//! Session / launch lifecycle split out of `app_runtime/mod.rs` for
//! SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - The launch payload types ([`ProcessLaunch`], [`AgentLaunchCompletion`],
//!   [`AgentLaunchResult`]) and the success dispatch bridge
//!   (`dispatch_agent_launch_success`)
//! - [`LaunchWizardMemoryCache`] (session cache backing the Launch Wizard)
//!   and `launch_config_from_persisted_session`
//! - SPEC-2809 launch stage correlation (`next_agent_launch_stage_id`
//!   fed by the `AppRuntime::agent_launch_stage_counter` field per
//!   SPEC-3064 FR-002, `emit_agent_launch_stage`) and the
//!   SPEC-2359 in-flight launch dedup (`INFLIGHT_LAUNCH_TTL`,
//!   `inflight_launch_key`)
//! - Issue<->branch link persistence (`IssueBranchLinkStore`,
//!   `record_issue_branch_link_with_cache_dir`, ...) and the codex managed
//!   hook discovery / trust registration helpers
//! - The launch / spawn / close-work method surface
//!   ([`AppRuntime::handle_launch_complete`], [`AppRuntime::start_window`],
//!   [`AppRuntime::spawn_agent_window_async`], [`AppRuntime::close_work`],
//!   [`AppRuntime::mark_agent_session_stopped`], ...)
//!
//! Behavior-preserving move: `WindowRuntime`, `LaunchWizardSession`, and
//! `AppRuntime::new` stay in `mod.rs` and are reached via `super`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use super::continuation::{
    pending_execution_activation_status, pending_fresh_execution_activation_status,
};
use super::{
    active_agent_session_matches_work, agent_launch_purpose_title,
    apply_docker_runtime_to_launch_config, apply_host_package_runner_fallback_checked,
    apply_windows_host_shell_wrapper, combined_window_id, detect_shell_program,
    finalize_docker_agent_launch_config_with_runtime, geometry_to_pty_size,
    install_launch_gwt_bin_env, intake_hook_config_is_disposable, is_ephemeral_intake_worktree,
    launch_output_mirror, mark_auto_resume_source_completed, normalize_branch_name,
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode,
    resolve_launch_spec_with_fallback, resolve_launch_worktree, same_worktree_path,
    save_resumed_workspace_projection, save_start_work_workspace_projection, ActiveAgentSession,
    AgentCapabilityIssuer, AgentKanbanLaunchTarget, AppEventProxy, AppRuntime, BackendEvent,
    LaunchFeedbackContext, LiveSessionEntry, OutboundEvent, Pane, PendingContinueWork,
    PendingFreshExecutionLaunch, UserEvent, WindowGeometry, WindowPreset, WindowProcessStatus,
    WindowRuntime, WorkspaceResumeContext,
};

#[derive(Clone)]
pub struct ProcessLaunch {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) remove_env: Vec<String>,
    pub(crate) cwd: Option<PathBuf>,
}

fn private_launch_env_key(key: &str) -> bool {
    matches!(
        key,
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV
            | gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV
            | gwt_agent::GWT_SESSION_ID_ENV
            | gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV
    )
}

fn private_launch_env_assignment(argument: &str) -> Option<&str> {
    let (key, _) = argument.split_once('=')?;
    private_launch_env_key(key).then_some(key)
}

#[allow(clippy::too_many_arguments)]
fn pending_fresh_execution_launch_from_session(
    sessions_dir: &Path,
    session_id: &str,
    worktree_path: &Path,
    agent_project_root: &str,
    linked_issue_number: Option<u64>,
    base_branch: Option<String>,
    resume_context: Option<WorkspaceResumeContext>,
    launch_feedback_context: Option<LaunchFeedbackContext>,
    readiness_nonce: &str,
) -> Result<PendingFreshExecutionLaunch, String> {
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    let session = gwt_agent::Session::load(&session_path).map_err(|error| {
        format!(
            "fresh linked-owner launch Session could not be read at {}: {error}",
            session_path.display()
        )
    })?;
    if session.id != session_id || !same_worktree_path(&session.worktree_path, worktree_path) {
        return Err(
            "fresh linked-owner launch Session does not match its materialized worktree"
                .to_string(),
        );
    }
    let binding = session.execution_binding.clone().ok_or_else(|| {
        "fresh linked-owner launch Session has no Prepared execution binding".to_string()
    })?;
    if binding.session_id != session_id
        || session.linked_issue_number != Some(binding.owner_number)
        || linked_issue_number != Some(binding.owner_number)
    {
        return Err(
            "fresh linked-owner launch owner does not match its persisted Session binding"
                .to_string(),
        );
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return Err("fresh linked-owner launch owner kind is not canonical".to_string()),
    };
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    let ledger = gwt::cli::execution_state::load_generation_ledger(worktree_path, owner)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "fresh linked-owner launch owner ledger is missing".to_string())?;
    if ledger.current_effective_status()
        != Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
    {
        return Err(
            "fresh linked-owner launch no longer has a terminal Blocked predecessor".to_string(),
        );
    }
    let attempt = gwt::cli::execution_state::prepared_fresh_linked_owner_launch_for_session(
        worktree_path,
        owner,
        session_id,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| {
        "fresh linked-owner launch has no unique Prepared successor attempt".to_string()
    })?;
    if !gwt::cli::execution_state::prepared_execution_binding_matches(
        worktree_path,
        owner,
        session_id,
        &binding.identity,
    )
    .map_err(|error| error.to_string())?
    {
        return Err(
            "fresh linked-owner launch Session does not match its Prepared successor".to_string(),
        );
    }
    let predecessor_binding =
        gwt::cli::execution_state::current_execution_binding(worktree_path, owner)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "fresh linked-owner launch predecessor binding is missing".to_string()
            })?;
    if predecessor_binding == binding.identity {
        return Err(
            "fresh linked-owner launch candidate is already the current generation".to_string(),
        );
    }
    let project_root = session
        .project_state_root
        .clone()
        .or_else(|| {
            (!agent_project_root.trim().is_empty()).then(|| PathBuf::from(agent_project_root))
        })
        .ok_or_else(|| {
            "fresh linked-owner launch canonical Project State root is missing".to_string()
        })?;
    Ok(PendingFreshExecutionLaunch {
        operation_id: attempt.request.operation_id.clone(),
        project_root,
        worktree_path: worktree_path.to_path_buf(),
        owner,
        request: attempt.request,
        binding,
        readiness_nonce: readiness_nonce.to_string(),
        predecessor_binding,
        base_branch,
        linked_issue_number,
        resume_context,
        launch_feedback_context,
    })
}

fn rollback_materialized_fresh_execution_launch(
    sessions_dir: &Path,
    session_id: &str,
    worktree_path: &Path,
    reason: &str,
) -> Result<(), String> {
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    let session = gwt_agent::Session::load(&session_path).map_err(|error| error.to_string())?;
    let binding = session.execution_binding.clone().ok_or_else(|| {
        "fresh linked-owner rollback cannot prove the candidate Session binding".to_string()
    })?;
    if session.id != session_id
        || binding.session_id != session_id
        || !same_worktree_path(&session.worktree_path, worktree_path)
    {
        return Err("fresh linked-owner rollback candidate identity no longer matches".to_string());
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => {
            return Err("fresh linked-owner rollback candidate owner is not canonical".to_string())
        }
    };
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    let attempt = gwt::cli::execution_state::prepared_fresh_linked_owner_launch_for_session(
        worktree_path,
        owner,
        session_id,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "fresh linked-owner rollback found no unique Prepared candidate".to_string())?;
    if !gwt::cli::execution_state::prepared_execution_binding_matches(
        worktree_path,
        owner,
        session_id,
        &binding.identity,
    )
    .map_err(|error| error.to_string())?
    {
        return Err(
            "fresh linked-owner rollback candidate binding is no longer Prepared".to_string(),
        );
    }
    gwt::cli::execution_state::abort_successor(worktree_path, owner, &attempt.request, reason)
        .map_err(|error| error.to_string())?;
    gwt_agent::remove_session_if_execution_binding_matches(sessions_dir, session_id, &binding)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

impl std::fmt::Debug for ProcessLaunch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_env = self
            .env
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str(),
                    if private_launch_env_key(key) {
                        "<redacted>"
                    } else {
                        value.as_str()
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let redacted_args = self
            .args
            .iter()
            .map(|argument| {
                private_launch_env_assignment(argument)
                    .map(|key| format!("{key}=<redacted>"))
                    .unwrap_or_else(|| argument.clone())
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("ProcessLaunch")
            .field("command", &self.command)
            .field("args", &redacted_args)
            .field("env", &redacted_env)
            .field("remove_env", &self.remove_env)
            .field("cwd", &self.cwd)
            .finish()
    }
}

fn install_agent_capability_env(
    env: &mut HashMap<String, String>,
    issuer: Option<&AgentCapabilityIssuer>,
    project_root: &Path,
    session_id: &str,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    container_runtime: Option<&gwt_docker::detect::ResolvedContainerRuntime>,
) -> Result<(), String> {
    install_agent_capability_env_with_binding(
        env,
        issuer,
        project_root,
        session_id,
        None,
        runtime_target,
        container_runtime,
    )
}

fn install_agent_capability_env_with_binding(
    env: &mut HashMap<String, String>,
    issuer: Option<&AgentCapabilityIssuer>,
    project_root: &Path,
    session_id: &str,
    execution_binding: Option<&gwt_agent::SessionExecutionBinding>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    container_runtime: Option<&gwt_docker::detect::ResolvedContainerRuntime>,
) -> Result<(), String> {
    let Some(issuer) = issuer else {
        return Ok(());
    };
    let endpoints = preflight_agent_capability_endpoints(
        issuer,
        project_root,
        session_id,
        runtime_target,
        container_runtime,
    )?;
    issue_preflighted_agent_capability_env(
        env,
        issuer,
        project_root,
        session_id,
        execution_binding.map_or(
            AgentCapabilityLaunchAuthority::Inspection,
            AgentCapabilityLaunchAuthority::Active,
        ),
        endpoints,
    )
}

struct PreflightedAgentCapabilityEndpoints {
    forward_url: String,
    pane_websocket_url: String,
}

fn preflight_agent_capability_endpoints(
    issuer: &AgentCapabilityIssuer,
    project_root: &Path,
    session_id: &str,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    container_runtime: Option<&gwt_docker::detect::ResolvedContainerRuntime>,
) -> Result<PreflightedAgentCapabilityEndpoints, String> {
    issuer.preflight_issue(project_root, session_id)?;
    let runtime_kind = container_runtime.map(gwt_docker::detect::ResolvedContainerRuntime::kind);
    let pane_websocket_url = gwt_agent::pane_websocket_url_for_launch_runtime(
        issuer.pane_websocket_url(),
        issuer.agent_pane_websocket_url(),
        runtime_target,
        runtime_kind,
    )?;
    let forward_url = gwt_agent::hook_forward_url_for_launch_runtime(
        issuer.hook_forward_url(),
        runtime_target,
        runtime_kind,
    )?;
    Ok(PreflightedAgentCapabilityEndpoints {
        forward_url,
        pane_websocket_url,
    })
}

fn issue_preflighted_agent_capability_env(
    env: &mut HashMap<String, String>,
    issuer: &AgentCapabilityIssuer,
    project_root: &Path,
    session_id: &str,
    execution_authority: AgentCapabilityLaunchAuthority<'_>,
    endpoints: PreflightedAgentCapabilityEndpoints,
) -> Result<(), String> {
    let target = match execution_authority {
        AgentCapabilityLaunchAuthority::Inspection => issuer.issue(project_root, session_id)?,
        AgentCapabilityLaunchAuthority::Prepared(binding) => {
            issuer.issue_prepared(project_root, session_id, binding.clone())?
        }
        AgentCapabilityLaunchAuthority::Active(binding) => {
            issuer.issue_bound(project_root, session_id, binding.clone())?
        }
    };
    env.insert(
        gwt_agent::GWT_HOOK_FORWARD_URL_ENV.to_string(),
        endpoints.forward_url,
    );
    env.insert(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
        target.token,
    );
    env.insert(
        gwt_agent::GWT_PANE_WS_URL_ENV.to_string(),
        endpoints.pane_websocket_url,
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum AgentCapabilityLaunchAuthority<'a> {
    Inspection,
    Prepared(&'a gwt_agent::SessionExecutionBinding),
    Active(&'a gwt_agent::SessionExecutionBinding),
}

struct FinalizedAgentCapabilityLaunch<'a> {
    issuer: Option<&'a AgentCapabilityIssuer>,
    sessions_dir: &'a Path,
    session: &'a mut gwt_agent::Session,
    project_root: &'a Path,
    worktree: &'a Path,
    producing_owner: Option<gwt::cli::execution_state::ExecutionOwnerKey>,
    prepared_continuation: Option<&'a gwt_agent::SessionExecutionBinding>,
    execution_entrypoint: &'a str,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    container_runtime: Option<&'a gwt_docker::detect::ResolvedContainerRuntime>,
}

impl FinalizedAgentCapabilityLaunch<'_> {
    fn install(self, env: &mut HashMap<String, String>) -> Result<(), String> {
        let Self {
            issuer,
            sessions_dir,
            session,
            project_root,
            worktree,
            producing_owner,
            prepared_continuation,
            execution_entrypoint,
            runtime_target,
            container_runtime,
        } = self;
        if let Some(binding) = prepared_continuation {
            if producing_owner.is_some() {
                return Err(
                    "Prepared continuation cannot enter the genesis execution launch path"
                        .to_string(),
                );
            }
            let issuer = issuer.ok_or_else(|| {
                "Prepared continuation is missing its Host capability issuer".to_string()
            })?;
            if session.execution_binding.as_ref() != Some(binding) {
                return Err(
                    "Prepared continuation Session binding changed before capability issuance"
                        .to_string(),
                );
            }
            let owner_kind = match binding.owner_kind.as_str() {
                "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
                "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                _ => return Err("Prepared continuation owner kind is not canonical".to_string()),
            };
            let owner = gwt::cli::execution_state::ExecutionOwnerKey {
                kind: owner_kind,
                number: binding.owner_number,
            };
            if !gwt::cli::execution_state::prepared_execution_binding_matches(
                worktree,
                owner,
                &session.id,
                &binding.identity,
            )
            .map_err(|error| error.to_string())?
            {
                return Err(
                    "Prepared continuation no longer matches its owner generation attempt"
                        .to_string(),
                );
            }
            let endpoints = preflight_agent_capability_endpoints(
                issuer,
                project_root,
                &session.id,
                runtime_target,
                container_runtime,
            )?;
            return issue_preflighted_agent_capability_env(
                env,
                issuer,
                project_root,
                &session.id,
                AgentCapabilityLaunchAuthority::Prepared(binding),
                endpoints,
            );
        }
        let Some(owner) = producing_owner else {
            return install_agent_capability_env(
                env,
                issuer,
                project_root,
                &session.id,
                runtime_target,
                container_runtime,
            );
        };
        let issuer = issuer.ok_or_else(|| {
            "producing launch is missing its Host capability issuer; no execution was materialized"
                .to_string()
        })?;
        let endpoints = preflight_agent_capability_endpoints(
            issuer,
            project_root,
            &session.id,
            runtime_target,
            container_runtime,
        )?;

        let mut current_binding =
            gwt::cli::execution_state::current_execution_binding(worktree, owner)
                .map_err(|error| error.to_string())?;
        if current_binding.is_none()
            && gwt::cli::execution_state::load(worktree)
                .map_err(|error| error.to_string())?
                .is_some_and(|legacy| {
                    legacy.owner_kind == owner.kind
                        && legacy.owner_number == owner.number
                        && legacy.status
                            == gwt::cli::execution_state::ExecutionControlStatus::Blocked
                })
        {
            // A pre-generation terminal ECR is still authoritative audit
            // evidence. Import its exact bytes before any launch writer can
            // materialize a new Active record over it; the ordinary Blocked
            // successor transaction below then owns readiness/activation.
            gwt::cli::execution_state::ensure_generation_ledger(
                worktree,
                owner,
                gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
            )
            .map_err(|error| error.to_string())?;
            current_binding = gwt::cli::execution_state::current_execution_binding(worktree, owner)
                .map_err(|error| error.to_string())?;
        }
        if current_binding.is_some() {
            let ledger = gwt::cli::execution_state::load_generation_ledger(worktree, owner)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    "the current execution binding has no integrity-valid owner ledger".to_string()
                })?;
            if ledger.current_effective_status()
                != Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
            {
                return Err(
                    "an execution generation already exists; use Continue work to create a successor"
                        .to_string(),
                );
            }

            let repo_hash = session
                .repo_hash
                .clone()
                .filter(|repo_hash| !repo_hash.trim().is_empty())
                .ok_or_else(|| {
                    "producing Session is missing its canonical repository hash".to_string()
                })?;
            let request = gwt::cli::execution_state::SuccessorRequest {
                operation_id: format!("fresh-launch-{}", uuid::Uuid::new_v4()),
                principal_id: "gwt-host-launch".to_string(),
                work_id: None,
                source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
                session_binding_id: uuid::Uuid::new_v4().to_string(),
                initial_session_id: session.id.clone(),
                entrypoint: execution_entrypoint.to_string(),
                requested_at: chrono::Utc::now(),
            };
            gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
                worktree, owner, &request,
            )
            .map_err(|error| error.to_string())?;
            let abort_prepared = |reason: &str| {
                gwt::cli::execution_state::abort_successor(worktree, owner, &request, reason)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            };
            let identity = match gwt::cli::execution_state::prepared_successor_execution_binding(
                worktree, owner, &request,
            ) {
                Ok(identity) => identity,
                Err(error) => {
                    let abort = abort_prepared("fresh launch binding derivation failed");
                    return Err(match abort {
                        Ok(()) => error.to_string(),
                        Err(abort_error) => format!(
                            "{error}; failed to abort fresh launch successor: {abort_error}"
                        ),
                    });
                }
            };
            let binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session.id.clone(),
                repo_hash,
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            };
            if let Err(error) = session.set_execution_binding(Some(binding.clone())) {
                let abort = abort_prepared("fresh launch Session binding failed");
                return Err(match abort {
                    Ok(()) => error,
                    Err(abort_error) => {
                        format!("{error}; failed to abort fresh launch successor: {abort_error}")
                    }
                });
            }
            if let Err(error) = session.save(sessions_dir) {
                let _ = session.set_execution_binding(None);
                let abort = abort_prepared("fresh launch Session persistence failed");
                return Err(match abort {
                    Ok(()) => format!("failed to persist fresh launch Session binding: {error}"),
                    Err(abort_error) => format!(
                        "failed to persist fresh launch Session binding: {error}; failed to abort fresh launch successor: {abort_error}"
                    ),
                });
            }
            if let Err(error) = issue_preflighted_agent_capability_env(
                env,
                issuer,
                project_root,
                &session.id,
                AgentCapabilityLaunchAuthority::Prepared(&binding),
                endpoints,
            ) {
                let rollback = session.set_execution_binding(None).and_then(|()| {
                    session
                        .save(sessions_dir)
                        .map_err(|error| error.to_string())
                });
                let abort = abort_prepared("fresh launch capability issuance failed");
                return Err(match (rollback, abort) {
                    (Ok(()), Ok(())) => error,
                    (rollback, abort) => format!(
                        "{error}; fresh launch rollback failed (Session: {}; successor: {})",
                        rollback.err().unwrap_or_else(|| "ok".to_string()),
                        abort.err().unwrap_or_else(|| "ok".to_string()),
                    ),
                });
            }
            env.insert(
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.to_string(),
                uuid::Uuid::new_v4().to_string(),
            );
            return Ok(());
        }

        gwt::cli::execution_state::materialize_at_launch(
            worktree,
            owner.kind,
            owner.number,
            &session.id,
            execution_entrypoint,
            false,
        )
        .map_err(|error| error.to_string())?;
        gwt::cli::execution_state::ensure_generation_ledger(
            worktree,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .map_err(|error| error.to_string())?;
        let identity = gwt::cli::execution_state::current_execution_binding(worktree, owner)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "execution generation materialization did not publish a current binding".to_string()
            })?;
        let repo_hash = session
            .repo_hash
            .clone()
            .filter(|repo_hash| !repo_hash.trim().is_empty())
            .ok_or_else(|| {
                "producing Session is missing its canonical repository hash".to_string()
            })?;
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash,
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        session.set_execution_binding(Some(binding.clone()))?;
        if let Err(error) = session.save(sessions_dir) {
            let _ = session.set_execution_binding(None);
            return Err(format!(
            "failed to persist the producing Session binding after its execution generation was materialized: {error}; transactional recovery is required before retry"
        ));
        }

        match issue_preflighted_agent_capability_env(
            env,
            issuer,
            project_root,
            &session.id,
            AgentCapabilityLaunchAuthority::Active(&binding),
            endpoints,
        ) {
            Ok(()) => Ok(()),
            Err(error) => {
                let rollback_result = session.set_execution_binding(None).and_then(|()| {
                    session
                        .save(sessions_dir)
                        .map_err(|error| error.to_string())
                });
                match rollback_result {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(format!(
                        "{error}; failed to roll back Session execution binding: {rollback_error}"
                    )),
                }
            }
        }
    }
}

fn persist_finalized_launch_session(
    sessions_dir: &Path,
    runtime_path: &Path,
    session: &mut gwt_agent::Session,
    docker_runtime_worktree: Option<&str>,
) -> Result<(), String> {
    if let Some(runtime_worktree) = docker_runtime_worktree {
        let project_state_root = session
            .project_state_root
            .as_deref()
            .filter(|root| !root.as_os_str().is_empty())
            .ok_or_else(|| {
                "Docker launch is missing the host Project State root before Session persistence"
                    .to_string()
            })?
            .to_path_buf();
        session.bind_docker_runtime(runtime_worktree, &project_state_root)?;
    }

    session
        .save(sessions_dir)
        .map_err(|error| error.to_string())?;
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
        .save(runtime_path)
        .map_err(|error| error.to_string())
}

pub type AgentLaunchCompletion = (
    ProcessLaunch,
    String,
    String,
    String,
    PathBuf,
    gwt_agent::AgentId,
    Option<u64>,
    Option<String>,
    gwt_agent::LaunchRuntimeTarget,
    AgentLaunchDisposition,
    String,
);

pub type AgentLaunchResult = Result<AgentLaunchCompletion, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentLaunchDisposition {
    WorkProducing,
    Inspection,
}

fn launch_disposition(config: &gwt_agent::LaunchConfig) -> AgentLaunchDisposition {
    match (&config.execution_intent, config.session_mode) {
        (gwt_agent::ExecutionLaunchIntent::PreparedContinuation(_), _) => {
            AgentLaunchDisposition::WorkProducing
        }
        (
            gwt_agent::ExecutionLaunchIntent::Automatic,
            gwt_agent::SessionMode::Resume | gwt_agent::SessionMode::Continue,
        ) => AgentLaunchDisposition::Inspection,
        (gwt_agent::ExecutionLaunchIntent::Automatic, gwt_agent::SessionMode::Normal) => {
            AgentLaunchDisposition::WorkProducing
        }
    }
}

pub(super) fn dispatch_agent_launch_success<F>(
    proxy: AppEventProxy,
    window_id: String,
    completion: AgentLaunchCompletion,
    spawn_project_index_bootstrap: F,
) where
    F: FnOnce(AppEventProxy, PathBuf),
{
    let project_index_root = completion.4.clone();
    proxy.send(UserEvent::LaunchComplete {
        window_id,
        result: Ok(completion),
    });
    spawn_project_index_bootstrap(proxy, project_index_root);
}

pub(super) fn launch_config_from_persisted_session(
    session: &gwt_agent::Session,
) -> gwt_agent::LaunchConfig {
    let agent_id = session.agent_id.clone();
    let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id);
    builder = builder.working_dir(session.worktree_path.clone());
    if !session.branch.is_empty() {
        builder = builder.branch(session.branch.clone());
    }
    if let Some(model) = session.model.clone() {
        builder = builder.model(model);
    }
    if let Some(version) = session.tool_version.clone() {
        builder = builder.version(version);
    }
    if let Some(level) = session.reasoning_level.clone() {
        builder = builder.reasoning_level(level);
    }
    if session.skip_permissions {
        builder = builder.skip_permissions(true);
    }
    if session.fast_mode_enabled() {
        builder = builder.fast_mode(true);
    }
    builder = builder.runtime_target(session.runtime_target);
    if let Some(service) = session.docker_service.clone() {
        builder = builder.docker_service(service);
    }
    builder = builder.docker_lifecycle_intent(session.docker_lifecycle_intent);
    if let Some(shell) = session.windows_shell {
        builder = builder.windows_shell(shell);
    }
    if let Some(linked) = session.linked_issue_number {
        builder = builder.linked_issue_number(linked);
    }

    if let Some(resume_id) = session.exact_resume_session_id() {
        builder = builder
            .session_mode(gwt_agent::SessionMode::Resume)
            .resume_session_id(resume_id.to_string());
    } else {
        builder = builder.session_mode(gwt_agent::SessionMode::Normal);
    }

    let mut config = builder.build();
    if let Some(version) = session.tool_version.clone() {
        config.tool_version = Some(version);
    }
    if !session.display_name.is_empty() {
        config.display_name = session.display_name.clone();
    }
    config
}

#[derive(Debug, Clone)]
enum AgentWindowPlacement {
    Centered(WindowGeometry),
    Exact(WindowGeometry),
}

impl AgentWindowPlacement {
    fn bounds(&self) -> WindowGeometry {
        match self {
            Self::Centered(bounds) | Self::Exact(bounds) => bounds.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct AgentWindowSpawnOptions {
    placement: AgentWindowPlacement,
    workspace_resume_context: Option<WorkspaceResumeContext>,
    launch_feedback_context: Option<LaunchFeedbackContext>,
    agent_kanban_target: Option<AgentKanbanLaunchTarget>,
    continuation: Option<PendingContinueWork>,
}

#[derive(Debug, Clone)]
pub struct LaunchWizardMemoryCache {
    sessions: Vec<gwt_agent::Session>,
    agent_options: Vec<gwt::AgentOption>,
    // SPEC-3170 FR-001: Claude capability detection may read settings and run
    // `claude --version` once per process. The wizard stores the booleans at
    // cache load time and reuses them on every open.
    claude_ultracode_supported: bool,
    claude_workflows_enabled: bool,
}

impl LaunchWizardMemoryCache {
    pub(crate) fn load(sessions_dir: &Path) -> Self {
        let claude_capabilities = gwt_agent::claude_capability_snapshot();
        Self {
            sessions: Self::load_sessions(sessions_dir),
            agent_options: Self::load_agent_options(),
            claude_ultracode_supported: claude_capabilities.ultracode_supported,
            claude_workflows_enabled: claude_capabilities.workflows_enabled,
        }
    }

    #[cfg(test)]
    pub(crate) fn load_with_agent_options(
        sessions_dir: &Path,
        agent_options: Vec<gwt::AgentOption>,
    ) -> Self {
        Self::load_with_agent_options_and_capabilities(sessions_dir, agent_options, false, false)
    }

    #[cfg(test)]
    pub(crate) fn load_with_agent_options_and_capabilities(
        sessions_dir: &Path,
        agent_options: Vec<gwt::AgentOption>,
        claude_ultracode_supported: bool,
        claude_workflows_enabled: bool,
    ) -> Self {
        Self {
            sessions: Self::load_sessions(sessions_dir),
            agent_options,
            claude_ultracode_supported,
            claude_workflows_enabled,
        }
    }

    fn load_sessions(sessions_dir: &Path) -> Vec<gwt_agent::Session> {
        let Ok(entries) = std::fs::read_dir(sessions_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|ext| ext.to_str()) == Some("toml")).then_some(path)
            })
            .filter_map(|path| gwt_agent::Session::load_and_migrate(&path).ok())
            .collect()
    }

    fn load_agent_options() -> Vec<gwt::AgentOption> {
        gwt::load_agent_options(&gwt_agent::VersionCache::load(
            &gwt::default_wizard_version_cache_path(),
        ))
    }

    pub(super) fn refresh_agent_options(&mut self) {
        self.agent_options = Self::load_agent_options();
    }

    pub(super) fn agent_options(&self) -> Vec<gwt::AgentOption> {
        self.agent_options.clone()
    }

    /// SPEC-3170 FR-001: cached `claude --version`-derived ultracode capability,
    /// resolved once at load time so wizard open never re-spawns the probe.
    pub(super) fn claude_ultracode_supported(&self) -> bool {
        self.claude_ultracode_supported
    }

    /// SPEC-3170 FR-001: cached Claude dynamic-workflows capability, resolved
    /// once at load time so wizard open never re-reads the settings file.
    pub(super) fn claude_workflows_enabled(&self) -> bool {
        self.claude_workflows_enabled
    }

    pub(super) fn quick_start_entries(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Vec<gwt::QuickStartEntry> {
        gwt::launch_wizard::quick_start_entries_from_sessions(
            repo_path,
            branch_name,
            &self.sessions,
        )
    }

    fn latest_resumable_branch_session(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Option<gwt_agent::Session> {
        let entry = self
            .quick_start_entries(repo_path, branch_name)
            .into_iter()
            .find(|entry| entry.resume_session_id.is_some())?;
        self.sessions
            .iter()
            .find(|session| session.id == entry.session_id)
            .cloned()
    }

    /// Replace all cached sessions with a freshly disk-loaded set. Called from
    /// the off-thread branch load (#2995) so resume availability and resolution
    /// observe session TOMLs the hook CLI wrote out-of-process after launch,
    /// without ever blocking the main UI thread on disk I/O.
    fn replace_sessions(&mut self, sessions: Vec<gwt_agent::Session>) {
        self.sessions = sessions;
    }

    fn session_by_id(&self, session_id: &str) -> Option<&gwt_agent::Session> {
        self.sessions
            .iter()
            .find(|session| session.id == session_id)
    }

    pub(super) fn agent_preferences(&self) -> gwt::LaunchWizardPreviousProfiles {
        gwt::launch_wizard::previous_launch_profiles_from_sessions(&self.sessions)
    }

    pub(super) fn previous_profiles(&self, repo_path: &Path) -> gwt::LaunchWizardPreviousProfiles {
        gwt::launch_wizard::previous_launch_profiles_for_repo_from_sessions(
            repo_path,
            &self.sessions,
        )
    }

    fn record_session(&mut self, session: gwt_agent::Session) {
        if let Some(existing) = self
            .sessions
            .iter_mut()
            .find(|existing| existing.id == session.id)
        {
            *existing = session;
        } else {
            self.sessions.push(session);
        }
    }

    pub(super) fn mark_stopped(&mut self, session_id: &str) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.update_status(gwt_agent::AgentStatus::Stopped);
        }
    }
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
pub(super) struct IssueBranchLinkStore {
    #[serde(default)]
    pub(super) branches: HashMap<String, u64>,
}

/// SPEC-2809 — per-spawn correlation id for Launch Wizard stages so the
/// Console window's `agent` tab can group multiple stage events (binary
/// resolve / env prep / worktree create / PTY handoff) under one
/// invocation header. Atomic so parallel wizard sessions do not collide.
/// SPEC-3064 FR-002: the counter is the per-instance
/// `AppRuntime::agent_launch_stage_counter` field threaded in by callers.
pub(crate) fn next_agent_launch_stage_id(counter: &std::sync::atomic::AtomicU64) -> u64 {
    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Emit a `gwt.process.summary` event for one Launch Wizard stage so the
/// Console window's `agent` tab surfaces the pipeline that ends in the
/// PTY spawn. Stage semantics (`start`, `done`, `error`) follow the same
/// vocabulary as the `spawn_logged` summary contract.
pub(crate) fn emit_agent_launch_stage(spawn_id: u64, stage: &str, detail: &str) {
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

fn launch_argv_summary(args: &[String]) -> String {
    format!("argc={}", args.len())
}

/// SPEC-2359 W-17 (FR-398): dedup window for launches that are past window
/// registration but not yet live. Entries also clear on launch completion.
const INFLIGHT_LAUNCH_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const CONTINUE_WORK_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Identity of a launch for in-flight dedup. Includes the agent and the
/// resume conversation so parallel restores of *different* Sessions on the
/// same Work (startup auto-resume) and multi-agent launches on one Work stay
/// allowed — only a re-request of the *same* launch dedupes. `None` when the
/// config carries neither a branch nor a working dir: such launches have no
/// stable Work identity and must never dedup against each other.
fn inflight_launch_key(tab_id: &str, config: &gwt_agent::LaunchConfig) -> Option<String> {
    let branch = config
        .branch
        .as_deref()
        .map(normalize_branch_name)
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

pub(super) fn record_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    issue_number: u64,
    cache_dir: &Path,
) -> Result<(), String> {
    update_issue_branch_link_with_cache_dir(repo_path, branch_name, Some(issue_number), cache_dir)
}

fn clear_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    cache_dir: &Path,
) -> Result<(), String> {
    update_issue_branch_link_with_cache_dir(repo_path, branch_name, None, cache_dir)
}

fn update_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    issue_number: Option<u64>,
    cache_dir: &Path,
) -> Result<(), String> {
    let branch_name = branch_name.trim();
    if branch_name.is_empty() {
        return Ok(());
    }
    let Some(repo_hash) = gwt::index_worker::detect_repo_hash(repo_path) else {
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

fn codex_hook_discovery_mode_for_launch_config(
    config: &gwt_agent::LaunchConfig,
) -> gwt_skills::CodexHookDiscoveryMode {
    if config.agent_id != gwt_agent::AgentId::Codex {
        return gwt_skills::CodexHookDiscoveryMode::WorkspaceHome;
    }
    if let Some(mode) =
        codex_hook_discovery_mode_from_selected_codex_version(config.tool_version.as_deref())
    {
        return mode;
    }
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return gwt_skills::CodexHookDiscoveryMode::Both;
    }
    detect_installed_codex_hook_discovery_mode(config)
        .unwrap_or(gwt_skills::CodexHookDiscoveryMode::Both)
}

pub(super) fn codex_hook_discovery_mode_from_selected_codex_version(
    version: Option<&str>,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let version = version?.trim();
    if version.is_empty() || version == "installed" {
        return None;
    }
    if version == "latest" {
        return Some(gwt_skills::CodexHookDiscoveryMode::WorkspaceHome);
    }
    codex_hook_discovery_mode_from_semver(version)
}

pub(super) fn codex_hook_discovery_mode_from_codex_version_output(
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
    Some(if version < boundary {
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal
    } else {
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome
    })
}

fn detect_installed_codex_hook_discovery_mode(
    config: &gwt_agent::LaunchConfig,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let mut request = gwt_core::process::ProcessPlanRequest::new(&config.command).arg("--version");
    for key in &config.remove_env {
        request = request.env_remove(key);
    }
    for (key, value) in &config.env_vars {
        request = request.env(key, value);
    }
    let mut command = match gwt_core::process::resolved_command(request) {
        Ok(command) => command,
        Err(error) => {
            tracing::warn!(
                command = %config.command,
                error = %error,
                "installed Codex version probe could not resolve a safe executable"
            );
            return None;
        }
    };
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push(' ');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    codex_hook_discovery_mode_from_codex_version_output(&text)
}

pub(super) fn maybe_register_codex_managed_hook_trust_for_launch(
    profile_config_path: &Path,
    worktree_path: &Path,
    agent_id: &gwt_agent::AgentId,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    docker_service: Option<&str>,
    codex_home: Option<&Path>,
    codex_hook_discovery_mode: gwt_skills::CodexHookDiscoveryMode,
) -> Result<Option<gwt_skills::CodexHookTrustReport>, String> {
    if agent_id != &gwt_agent::AgentId::Codex {
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

    match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => {
            let Some(codex_config_path) = codex_home
                .map(|home| home.join("config.toml"))
                .or_else(|| codex_config_path_for_profile_config(profile_config_path))
            else {
                tracing::warn!(
                    profile_config = %profile_config_path.display(),
                    "cannot derive Codex config path while preparing Codex hook trust; continuing launch"
                );
                return Ok(None);
            };
            match gwt_skills::register_codex_managed_hook_trust_for_mode(
                worktree_path,
                &codex_config_path,
                codex_hook_discovery_mode,
            ) {
                Ok(report) => Ok(Some(report)),
                Err(error) => {
                    tracing::warn!(
                        worktree = %worktree_path.display(),
                        codex_config = %codex_config_path.display(),
                        error = %error,
                        "failed to register gwt-managed Codex hook trust; continuing launch"
                    );
                    Ok(None)
                }
            }
        }
        gwt_agent::LaunchRuntimeTarget::Docker => {
            if let Err(error) = gwt_agent::register_codex_managed_hook_trust_in_docker(
                worktree_path,
                docker_service,
                codex_hook_discovery_mode,
            ) {
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

fn codex_config_path_for_profile_config(profile_config_path: &Path) -> Option<PathBuf> {
    let gwt_config_dir = profile_config_path.parent()?;
    if gwt_config_dir.file_name().and_then(|name| name.to_str()) != Some(".gwt") {
        return None;
    }
    Some(gwt_config_dir.parent()?.join(".codex").join("config.toml"))
}

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
        sessions: Vec<gwt_agent::Session>,
    ) {
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
        if let Some(session) = self.launch_wizard.as_mut() {
            if session.wizard_id == wizard_id {
                session.wizard.open_branch_candidates = candidates;
                return vec![self.launch_wizard_state_outbound()];
            }
        }
        Vec::new()
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

    pub(crate) fn handle_launch_complete(
        &mut self,
        window_id: String,
        result: AgentLaunchResult,
    ) -> Vec<OutboundEvent> {
        let is_continue_work = self.pending_continue_work.contains_key(&window_id);
        let workspace_resume_context = self.pending_workspace_resume_contexts.remove(&window_id);
        let launch_feedback_context = self.pending_launch_feedback_contexts.remove(&window_id);
        let auto_resume_source_session_id = self.pending_auto_resume_sources.remove(&window_id);
        self.inflight_launches
            .retain(|_, (pending_window_id, _)| pending_window_id != &window_id);
        match result {
            Ok((
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
                linked_issue_number,
                base_branch,
                runtime_target,
                launch_disposition,
                agent_project_root,
            )) => {
                let issued_capability_token = process_launch
                    .env
                    .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
                    .cloned();
                let fresh_readiness_nonce = process_launch
                    .env
                    .get(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV)
                    .cloned();
                let pending_fresh_execution = if is_continue_work {
                    None
                } else if let Some(readiness_nonce) = fresh_readiness_nonce.as_deref() {
                    if launch_disposition != AgentLaunchDisposition::WorkProducing {
                        self.revoke_unbound_agent_capability(issued_capability_token.as_deref());
                        return self.launch_error_events(
                            window_id,
                            "an inspection launch cannot carry fresh execution readiness"
                                .to_string(),
                            launch_feedback_context,
                        );
                    }
                    match pending_fresh_execution_launch_from_session(
                        &self.sessions_dir,
                        &session_id,
                        &worktree_path,
                        &agent_project_root,
                        linked_issue_number,
                        base_branch.clone(),
                        workspace_resume_context.clone(),
                        launch_feedback_context.clone(),
                        readiness_nonce,
                    ) {
                        Ok(pending) => Some(pending),
                        Err(error) => {
                            self.revoke_unbound_agent_capability(
                                issued_capability_token.as_deref(),
                            );
                            let rollback = rollback_materialized_fresh_execution_launch(
                                &self.sessions_dir,
                                &session_id,
                                &worktree_path,
                                "fresh launch readiness reconstruction failed",
                            );
                            let error = match rollback {
                                Ok(()) => error,
                                Err(rollback_error) => format!(
                                    "{error}; candidate rollback requires reconciliation: {rollback_error}"
                                ),
                            };
                            return self.launch_error_events(
                                window_id,
                                format!(
                                    "fresh linked-owner launch readiness could not be recovered: {error}"
                                ),
                                launch_feedback_context,
                            );
                        }
                    }
                } else {
                    None
                };
                let is_fresh_execution_launch = pending_fresh_execution.is_some();
                if let Some(pending) = pending_fresh_execution {
                    let operation_id = pending.operation_id.clone();
                    self.pending_fresh_execution_launches
                        .insert(window_id.clone(), pending);
                    let timeout_proxy = self.proxy.clone();
                    let timeout_window_id = window_id.clone();
                    thread::spawn(move || {
                        thread::sleep(CONTINUE_WORK_READY_TIMEOUT);
                        timeout_proxy.send(UserEvent::ContinueWorkReadyTimeout {
                            window_id: timeout_window_id,
                            operation_id,
                        });
                    });
                }
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    self.revoke_unbound_agent_capability(issued_capability_token.as_deref());
                    return self.launch_error_events_with_continue_work(
                        window_id,
                        "Window not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    self.revoke_unbound_agent_capability(issued_capability_token.as_deref());
                    return self.launch_error_events_with_continue_work(
                        window_id,
                        "Project tab not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                // SPEC-2359 W-16 (FR-387): a launch fetches origin refs, so
                // piggyback the cross-machine intake (30s throttle keeps
                // launch bursts cheap).
                self.spawn_work_events_ingest(tab.project_root.clone(), false);
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    self.revoke_unbound_agent_capability(issued_capability_token.as_deref());
                    return self.launch_error_events_with_continue_work(
                        window_id,
                        "Window not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                let tab_id = address.tab_id.clone();
                let project_root = tab.project_root.clone();
                let geometry = window.geometry.clone();
                let session_id_for_restore = session_id.clone();

                if let Some(token) = issued_capability_token {
                    if let Some(previous) = self
                        .agent_capability_tokens
                        .insert(window_id.clone(), token.clone())
                    {
                        if previous != token {
                            self.revoke_unbound_agent_capability(Some(&previous));
                        }
                    }
                }
                self.active_agent_sessions.insert(
                    window_id.clone(),
                    ActiveAgentSession {
                        window_id: window_id.clone(),
                        session_id,
                        agent_id: agent_id.to_string(),
                        branch_name,
                        display_name,
                        worktree_path: worktree_path.clone(),
                        agent_project_root,
                        runtime_target,
                        tab_id: tab_id.clone(),
                    },
                );
                if launch_disposition == AgentLaunchDisposition::Inspection {
                    self.inspection_agent_windows.insert(window_id.clone());
                } else {
                    self.inspection_agent_windows.remove(&window_id);
                }
                let _ = gwt_agent::persist_session_restore_window_on_startup(
                    &self.sessions_dir,
                    &session_id_for_restore,
                    true,
                );
                if let Some(tab) = self.tab_mut(&tab_id) {
                    let _ = tab
                        .workspace
                        .set_session_id(&address.raw_id, Some(session_id_for_restore.clone()));
                }
                if let Some(source_session_id) = auto_resume_source_session_id {
                    mark_auto_resume_source_completed(&self.sessions_dir, &source_session_id);
                }
                self.refresh_launch_wizard_session_cache(&window_id);

                // SPEC-2809 — Launch Wizard always spawns an AI agent
                // launch sequence (binary resolve / env prep / PTY
                // spawn) so the Console window's `agent` tab shows the
                // wizard pipeline up to the moment xterm.js takes over.
                let stage_id = next_agent_launch_stage_id(&self.agent_launch_stage_counter);
                emit_agent_launch_stage(
                    stage_id,
                    "resolve_binary",
                    &format!("wizard launch {}", process_launch.command),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "prepare_env",
                    &format!("worktree={}", worktree_path.display()),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "spawn_pty",
                    &launch_argv_summary(&process_launch.args),
                );
                match self.spawn_process_window_with_console_kind(
                    &window_id,
                    geometry,
                    process_launch,
                    Some(gwt_core::process_console::ProcessKind::AgentBootstrap),
                ) {
                    Ok(()) => {
                        emit_agent_launch_stage(stage_id, "ready", "PTY handoff complete");
                        if launch_disposition == AgentLaunchDisposition::WorkProducing
                            && !is_fresh_execution_launch
                        {
                            let linkage_result = match linked_issue_number {
                                Some(issue_number) => record_issue_branch_link_with_cache_dir(
                                    &worktree_path,
                                    &self.active_agent_sessions[&window_id].branch_name,
                                    issue_number,
                                    &self.issue_link_cache_dir,
                                ),
                                None => clear_issue_branch_link_with_cache_dir(
                                    &worktree_path,
                                    &self.active_agent_sessions[&window_id].branch_name,
                                    &self.issue_link_cache_dir,
                                ),
                            };
                            if let Err(error) = linkage_result {
                                tracing::warn!(
                                    worktree = %worktree_path.display(),
                                    branch = %self.active_agent_sessions[&window_id].branch_name,
                                    ?linked_issue_number,
                                    error = %error,
                                    "issue branch linkage update skipped after agent launch"
                                );
                            }
                        }
                        let mut workspace_projection_updated = false;
                        let live_session_ids: std::collections::HashSet<String> = self
                            .active_agent_sessions
                            .values()
                            .map(|session| session.session_id.clone())
                            .collect();
                        let active_session = &self.active_agent_sessions[&window_id];
                        if launch_disposition == AgentLaunchDisposition::WorkProducing
                            && !is_continue_work
                            && !is_fresh_execution_launch
                        {
                            if let Some(base_branch) = base_branch.as_deref() {
                                match save_start_work_workspace_projection(
                                    &project_root,
                                    active_session,
                                    base_branch,
                                    linked_issue_number,
                                    workspace_resume_context.as_ref(),
                                    &live_session_ids,
                                ) {
                                    Ok(()) => {
                                        workspace_projection_updated = true;
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            project_root = %project_root.display(),
                                            branch = %active_session.branch_name,
                                            error = %error,
                                            "workspace projection update skipped after Start Work launch"
                                        );
                                    }
                                }
                            } else if let Some(context) = workspace_resume_context.as_ref() {
                                match save_resumed_workspace_projection(
                                    &project_root,
                                    active_session,
                                    None,
                                    linked_issue_number,
                                    context,
                                    &live_session_ids,
                                ) {
                                    Ok(()) => {
                                        workspace_projection_updated = true;
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            project_root = %project_root.display(),
                                            branch = %active_session.branch_name,
                                            error = %error,
                                            "workspace projection update skipped after Workspace Resume launch"
                                        );
                                    }
                                }
                            }
                        }
                        let _ = self.persist();
                        self.launch_error_terminal_details.remove(&window_id);
                        let mut events = vec![self.workspace_state_broadcast()];
                        if workspace_projection_updated
                            && self.active_tab_id.as_deref() == Some(tab_id.as_str())
                        {
                            if let Some(tab) = self.tab(&tab_id) {
                                if let Some(projection) =
                                    self.active_work_projection_for_tab(&tab_id, tab)
                                {
                                    events.push(OutboundEvent::broadcast(
                                        BackendEvent::ActiveWorkProjection {
                                            projection: Box::new(projection),
                                        },
                                    ));
                                }
                            }
                        }
                        let composed_status = self
                            .window_status(&window_id)
                            .unwrap_or(WindowProcessStatus::Running);
                        events.extend(Self::status_events(
                            window_id.clone(),
                            composed_status,
                            is_fresh_execution_launch
                                .then(|| "Waiting for authenticated SessionStart...".to_string()),
                        ));
                        if !is_fresh_execution_launch {
                            if let Some(issue_number) = launch_feedback_context
                                .as_ref()
                                .and_then(|context| context.issue_monitor_issue_number)
                            {
                                events.extend(self.issue_monitor_launch_succeeded_events(
                                    issue_number,
                                    &window_id,
                                ));
                            }
                        }
                        events
                    }
                    Err(error) => {
                        self.revoke_agent_capability_for_window(&window_id);
                        self.launch_error_events_with_continue_work(
                            window_id,
                            error,
                            launch_feedback_context,
                        )
                    }
                }
            }
            Err(error) => self.launch_error_events_with_continue_work(
                window_id,
                error,
                launch_feedback_context,
            ),
        }
    }

    pub(super) fn launch_error_events_with_continue_work(
        &mut self,
        window_id: String,
        detail: String,
        launch_feedback_context: Option<LaunchFeedbackContext>,
    ) -> Vec<OutboundEvent> {
        // Activation is the irreversible continuation commit. Reconcile its
        // exact durable/live state before the generic launch-error path can
        // tear down the pane, Session, or capability needed for readback and
        // waiter fan-out. An uncertain readback intentionally leaves the
        // pending receipt intact for the correlated retry path.
        if let Some(pending) = self.pending_continue_work.get(&window_id) {
            match pending_execution_activation_status(pending) {
                Some(true) => {
                    return self.continue_work_launch_failed_events(&window_id, &detail);
                }
                Some(false) => {}
                None => return Vec::new(),
            }
        }
        if let Some(pending) = self.pending_fresh_execution_launches.get(&window_id) {
            match pending_fresh_execution_activation_status(pending) {
                Some(true) => {
                    return self.fresh_execution_launch_failed_events(&window_id, &detail);
                }
                Some(false) => {}
                None => return Vec::new(),
            }
        }
        let mut events =
            self.launch_error_events(window_id.clone(), detail.clone(), launch_feedback_context);
        events.extend(self.continue_work_launch_failed_events(&window_id, &detail));
        events.extend(self.fresh_execution_launch_failed_events(&window_id, &detail));
        events
    }

    pub(crate) fn handle_shell_launch_complete(
        &mut self,
        window_id: String,
        result: Result<ProcessLaunch, String>,
    ) -> Vec<OutboundEvent> {
        match result {
            Ok(process_launch) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        None,
                    );
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Project tab not found".to_string(),
                        None,
                    );
                };
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        None,
                    );
                };
                let geometry = window.geometry.clone();

                // SPEC-2809 (revised) — second Launch Wizard exit path
                // emits the same launch banner sequence as the primary
                // handler so the Console window's `agent` tab is
                // consistent regardless of which wizard outcome the user
                // came in through.
                let stage_id = next_agent_launch_stage_id(&self.agent_launch_stage_counter);
                emit_agent_launch_stage(
                    stage_id,
                    "resolve_binary",
                    &format!("wizard launch {}", process_launch.command),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "prepare_env",
                    &launch_argv_summary(&process_launch.args),
                );
                match self.spawn_process_window_with_console_kind(
                    &window_id,
                    geometry,
                    process_launch,
                    Some(gwt_core::process_console::ProcessKind::AgentBootstrap),
                ) {
                    Ok(()) => {
                        emit_agent_launch_stage(stage_id, "ready", "PTY handoff complete");
                        self.launch_error_terminal_details.remove(&window_id);
                        let mut events = vec![self.workspace_state_broadcast()];
                        let composed_status = self
                            .window_status(&window_id)
                            .unwrap_or(WindowProcessStatus::Running);
                        events.extend(Self::status_events(window_id, composed_status, None));
                        events
                    }
                    Err(error) => {
                        emit_agent_launch_stage(stage_id, "error", &error);
                        self.launch_error_events(window_id, error, None)
                    }
                }
            }
            Err(error) => self.launch_error_events(window_id, error, None),
        }
    }

    pub(crate) fn start_window(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        preset: WindowPreset,
        geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        self.register_window(tab_id, raw_id);
        let window_id = combined_window_id(tab_id, raw_id);
        if !preset.requires_process() {
            self.set_window_status(tab_id, raw_id, WindowProcessStatus::Running);
            return Self::status_events(window_id, WindowProcessStatus::Running, None);
        }

        let project_root = self
            .tab(tab_id)
            .map(|tab| tab.project_root.clone())
            .unwrap_or_else(|| PathBuf::from("."));

        let shell = match detect_shell_program() {
            Ok(shell) => shell,
            Err(error) => {
                let detail = error.to_string();
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), detail.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(detail));
            }
        };

        let launch = match resolve_launch_spec_with_fallback(preset, &shell) {
            Ok(launch) => launch,
            Err(error) => {
                let detail = error.to_string();
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), detail.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(detail));
            }
        };

        let effective_env = match self.active_profile_spawn_env() {
            Ok(env) => env,
            Err(error) => {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(error));
            }
        }
        .with_project_root(&project_root);
        let (env, remove_env) = effective_env.into_parts();

        // SPEC-2809 (revised) — Surface the launch pipeline for AI
        // agent presets (Codex / Claude / Gemini / Agent) so the Console
        // window's `agent` tab shows what gwt is doing leading up to the
        // PTY spawn. Plain `Shell` panes do not emit launch banners
        // because nothing distinguishes them from arbitrary terminals.
        let is_agent_preset = matches!(
            preset,
            WindowPreset::Claude | WindowPreset::Codex | WindowPreset::Agent
        );
        let console_kind =
            is_agent_preset.then_some(gwt_core::process_console::ProcessKind::AgentBootstrap);
        let stage_id =
            is_agent_preset.then(|| next_agent_launch_stage_id(&self.agent_launch_stage_counter));
        if let Some(id) = stage_id {
            emit_agent_launch_stage(
                id,
                "resolve_binary",
                &format!("{} ({})", preset.title(), launch.command),
            );
            emit_agent_launch_stage(
                id,
                "prepare_env",
                &format!("project_root={}", project_root.display()),
            );
            emit_agent_launch_stage(id, "spawn_pty", &launch_argv_summary(&launch.args));
        }
        match self.spawn_process_window_with_console_kind(
            &window_id,
            geometry,
            ProcessLaunch {
                command: launch.command,
                args: launch.args,
                env,
                remove_env,
                cwd: Some(project_root),
            },
            console_kind,
        ) {
            Ok(()) => {
                if let Some(id) = stage_id {
                    emit_agent_launch_stage(id, "ready", "PTY handoff complete");
                }
                let composed_status = self
                    .window_status(&window_id)
                    .unwrap_or(WindowProcessStatus::Running);
                Self::status_events(window_id, composed_status, None)
            }
            Err(error) => {
                if let Some(id) = stage_id {
                    emit_agent_launch_stage(id, "error", &error);
                }
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                Self::status_events(window_id, WindowProcessStatus::Error, Some(error))
            }
        }
    }

    pub(crate) fn spawn_process_window_with_console_kind(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        launch: ProcessLaunch,
        console_kind: Option<gwt_core::process_console::ProcessKind>,
    ) -> Result<(), String> {
        let (cols, rows) = geometry_to_pty_size(&geometry);
        let pane = Pane::new_with_spawn_config(
            id.to_string(),
            gwt_terminal::pty::SpawnConfig {
                command: launch.command,
                args: launch.args,
                cols,
                rows,
                env: launch.env,
                remove_env: launch.remove_env,
                cwd: launch.cwd,
            },
        )
        .map_err(|error| error.to_string())?;
        let pane = Arc::new(Mutex::new(pane));

        let output_thread = self.spawn_output_thread(id.to_string(), pane.clone(), console_kind);
        let status_thread = self.spawn_status_thread(id.to_string(), pane.clone());
        if let Some(address) = self.window_lookup.get(id).cloned() {
            self.window_pty_statuses
                .insert(id.to_string(), WindowProcessStatus::Running);
            self.window_hook_states.remove(id);
            self.set_window_status(
                &address.tab_id,
                &address.raw_id,
                WindowProcessStatus::Running,
            );
        }
        self.window_details.remove(id);
        // Publish the PTY handle to the WebSocket fast-path registry BEFORE
        // inserting the runtime so that the first `terminal_input` from the
        // frontend (which can arrive immediately after `TerminalStatus`) has a
        // target to write to. Registry holds a cloned `Arc<PtyHandle>`; the
        // real owner remains the `Mutex<Pane>` in `WindowRuntime`.
        self.register_pty_writer(id, &pane);
        self.runtimes.insert(
            id.to_string(),
            WindowRuntime {
                pane,
                output_thread: Some(output_thread),
                status_thread: Some(status_thread),
            },
        );
        Ok(())
    }

    pub(crate) fn spawn_agent_window(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                placement: AgentWindowPlacement::Centered(bounds),
                workspace_resume_context,
                launch_feedback_context: None,
                agent_kanban_target: None,
                continuation: None,
            },
        )
    }

    pub(crate) fn spawn_agent_window_with_feedback(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: LaunchFeedbackContext,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                placement: AgentWindowPlacement::Centered(bounds),
                workspace_resume_context,
                launch_feedback_context: Some(launch_feedback_context),
                agent_kanban_target: None,
                continuation: None,
            },
        )
    }

    pub(crate) fn spawn_agent_window_with_feedback_at_geometry(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        geometry: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: LaunchFeedbackContext,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                placement: AgentWindowPlacement::Exact(geometry),
                workspace_resume_context,
                launch_feedback_context: Some(launch_feedback_context),
                agent_kanban_target: None,
                continuation: None,
            },
        )
    }

    pub(crate) fn spawn_agent_window_in_agent_kanban(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: Option<LaunchFeedbackContext>,
        target: AgentKanbanLaunchTarget,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                placement: AgentWindowPlacement::Centered(bounds),
                workspace_resume_context,
                launch_feedback_context,
                agent_kanban_target: Some(target),
                continuation: None,
            },
        )
    }

    pub(crate) fn spawn_agent_window_at_geometry(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        geometry: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                placement: AgentWindowPlacement::Exact(geometry),
                workspace_resume_context,
                launch_feedback_context: None,
                agent_kanban_target: None,
                continuation: None,
            },
        )
    }

    pub(crate) fn spawn_continue_work_window(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: WorkspaceResumeContext,
        continuation: PendingContinueWork,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                placement: AgentWindowPlacement::Centered(bounds),
                workspace_resume_context: Some(workspace_resume_context),
                launch_feedback_context: None,
                agent_kanban_target: None,
                continuation: Some(continuation),
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
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let events = self.focus_window_events(window_id, bounds);
        if events.is_empty() {
            vec![self.workspace_state_broadcast()]
        } else {
            events
        }
    }

    fn spawn_agent_window_with_placement(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        options: AgentWindowSpawnOptions,
    ) -> Result<Vec<OutboundEvent>, String> {
        let AgentWindowSpawnOptions {
            placement,
            workspace_resume_context,
            launch_feedback_context,
            agent_kanban_target,
            continuation,
        } = options;
        if continuation.is_some() {
            if let Some(window_id) =
                self.pending_continue_work
                    .iter()
                    .find_map(|(window_id, pending)| {
                        continuation
                            .as_ref()
                            .is_some_and(|candidate| {
                                pending.operation_id == candidate.operation_id
                                    && pending.work_id == candidate.work_id
                            })
                            .then(|| window_id.clone())
                    })
            {
                return Ok(self
                    .focus_existing_live_work_agent_events(&window_id, Some(placement.bounds())));
            }
        }
        if continuation.is_none() {
            if let Some(window_id) = self.live_agent_window_for_work(
                tab_id,
                config.branch.as_deref(),
                config.working_dir.as_deref(),
            ) {
                return Ok(self
                    .focus_existing_live_work_agent_events(&window_id, Some(placement.bounds())));
            }
        }
        // SPEC-2359 W-17 (FR-398, Issue #3034): the live-window check above
        // only sees launches whose agent session is already live. A re-click
        // while the previous launch is still materializing (window registered,
        // session pending) must focus that pending window, not spawn a twin.
        let inflight_key = continuation
            .is_none()
            .then(|| inflight_launch_key(tab_id, &config))
            .flatten();
        {
            let window_lookup = &self.window_lookup;
            self.inflight_launches.retain(|_, (window_id, started)| {
                started.elapsed() < INFLIGHT_LAUNCH_TTL
                    && window_lookup.contains_key(window_id.as_str())
            });
        }
        if let Some(key) = inflight_key.as_deref() {
            if let Some((window_id, _)) = self.inflight_launches.get(key) {
                let window_id = window_id.clone();
                return Ok(self
                    .focus_existing_live_work_agent_events(&window_id, Some(placement.bounds())));
            }
        }
        // Resolve every synchronously fallible launch dependency before adding
        // a window. A Continue work dispatch error must leave no untracked pane
        // for its caller to clean up after durably aborting the attempt.
        let profile_config_path = self.profile_config_path()?;
        let issue_link_cache_dir = self.issue_link_cache_dir.clone();
        let tab = self
            .tab_mut(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let project_root_path = tab.project_root.clone();
        let project_root = project_root_path.display().to_string();
        let title = config.display_name.clone();
        let purpose_title = workspace_resume_context
            .as_ref()
            .and_then(WorkspaceResumeContext::purpose_title)
            .or_else(|| {
                agent_launch_purpose_title(
                    &project_root_path,
                    config.linked_issue_number,
                    config.branch.as_deref(),
                    &issue_link_cache_dir,
                )
            });
        let window = match placement {
            AgentWindowPlacement::Centered(bounds) => {
                tab.workspace
                    .add_window_with_title(WindowPreset::Agent, title, true, bounds)
            }
            AgentWindowPlacement::Exact(geometry) => tab
                .workspace
                .add_window_at_geometry_with_title(WindowPreset::Agent, title, true, geometry),
        };
        if let Some(purpose_title) = purpose_title {
            let _ = tab
                .workspace
                .set_purpose_title(&window.id, Some(purpose_title));
        }
        let _ = tab
            .workspace
            .set_agent_id(&window.id, config.agent_id.command().to_string());
        if let Some(target) = agent_kanban_target.as_ref() {
            let _ = tab.workspace.place_agent_window_in_kanban(
                &window.id,
                &target.board_id,
                target.lane_id,
                None,
            );
        }
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        self.window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
        self.window_hook_states.remove(&window_id);
        if let Some(key) = inflight_key {
            self.inflight_launches
                .insert(key, (window_id.clone(), std::time::Instant::now()));
        }

        let mut events = vec![self.workspace_state_broadcast()];
        let composed_status = self
            .window_status(&window_id)
            .unwrap_or(WindowProcessStatus::Running);
        events.extend(Self::status_events(
            window_id.clone(),
            composed_status,
            Some("Launching...".to_string()),
        ));

        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        let agent_capability_issuer = self.agent_capability_issuer.clone();
        if let Some(context) = workspace_resume_context {
            self.pending_workspace_resume_contexts
                .insert(window_id.clone(), context);
        }
        if let Some(context) = launch_feedback_context {
            self.pending_launch_feedback_contexts
                .insert(window_id.clone(), context);
        }
        if let Some(continuation) = continuation {
            let operation_id = continuation.operation_id.clone();
            self.pending_continue_work
                .insert(window_id.clone(), continuation);
            let timeout_proxy = proxy.clone();
            let timeout_window_id = window_id.clone();
            thread::spawn(move || {
                thread::sleep(CONTINUE_WORK_READY_TIMEOUT);
                timeout_proxy.send(UserEvent::ContinueWorkReadyTimeout {
                    window_id: timeout_window_id,
                    operation_id,
                });
            });
        }

        thread::spawn(move || {
            Self::spawn_agent_window_async(
                proxy,
                sessions_dir,
                project_root,
                window_id,
                config,
                profile_config_path,
                agent_capability_issuer,
            );
        });

        Ok(events)
    }

    pub(crate) fn spawn_agent_window_async(
        proxy: AppEventProxy,
        sessions_dir: PathBuf,
        project_root: String,
        window_id: String,
        mut config: gwt_agent::LaunchConfig,
        profile_config_path: PathBuf,
        agent_capability_issuer: Option<AgentCapabilityIssuer>,
    ) {
        // SPEC-2014 FR-139..142 — while a Docker launch prepares (preflight,
        // compose ps/up incl. image build, exec probes), mirror docker-kind
        // Process Console lines into the agent terminal. Host launches keep
        // their immediate-PTY behavior untouched (FR-142).
        let docker_output_mirror =
            (config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker).then(|| {
                launch_output_mirror::DockerLaunchOutputMirror::start(
                    proxy.clone(),
                    window_id.clone(),
                )
            });
        let mut issued_capability_token = None;
        let result = (|| {
            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Preparing worktree...".to_string(),
            });
            resolve_launch_worktree(Path::new(&project_root), &mut config)?;

            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Starting Docker service...".to_string(),
            });
            let container_runtime =
                apply_docker_runtime_to_launch_config(Path::new(&project_root), &mut config)?;

            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
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
            gwt_agent::LaunchEnvironment::from_active_profile(
                &profile_config_path,
                config.runtime_target,
            )?
            .with_project_root(&worktree_path)
            .apply_to_parts(&mut config.env_vars, &mut config.remove_env);
            let codex_hook_discovery_mode = codex_hook_discovery_mode_for_launch_config(&config);
            // SPEC-3247 FR-002: select lane-specific coordination guidance from
            // the launch's ephemeral intake flag (same source as the
            // GWT_SESSION_KIND env export in prepare.rs), so an intake session
            // materializes curation-framed guidance without Work-state
            // instructions.
            let session_kind = gwt_skills::SessionKind::from_is_ephemeral(config.is_ephemeral);
            // SPEC-3248 (hooks v2 P0): materialize the lane file — the
            // deterministic source of truth hooks read via the lane registry —
            // from the authoritative launch-time lane (is_ephemeral). Best
            // effort: a write failure must not block the launch, and hooks fall
            // back to the execution default when the file is absent.
            let _ = gwt_skills::write_lane_file(
                &worktree_path,
                gwt_skills::LaneRegistry::for_session_kind(session_kind),
            );
            refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
                &worktree_path,
                &config.agent_id,
                codex_hook_discovery_mode,
                session_kind,
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
            let codex_home = config.env_vars.get("CODEX_HOME").map(PathBuf::from);
            if let Some(report) = maybe_register_codex_managed_hook_trust_for_launch(
                &profile_config_path,
                &worktree_path,
                &config.agent_id,
                config.runtime_target,
                config.docker_service.as_deref(),
                codex_home.as_deref(),
                codex_hook_discovery_mode,
            )? {
                if !report.trusted_entries.is_empty() {
                    proxy.send(UserEvent::LaunchProgress {
                        window_id: window_id.clone(),
                        message: format!(
                            "Trusted {} gwt-managed Codex hooks.",
                            report.trusted_entries.len()
                        ),
                    });
                }
            }

            if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Host {
                let fallback_report = apply_host_package_runner_fallback_checked(&mut config)?;
                for message in fallback_report.messages {
                    proxy.send(UserEvent::LaunchProgress {
                        window_id: window_id.clone(),
                        message,
                    });
                }
            }
            install_launch_gwt_bin_env(&mut config.env_vars, config.runtime_target)?;
            // SPEC-3248 P8a: derive the execution entrypoint from the raw
            // launch argv BEFORE the Windows host shell wrapper rewrites it
            // (the `$gwt-*` prompt token moves into an env var / embedded
            // script on wrapped launches).
            let execution_entrypoint = gwt::cli::execution_state::entrypoint_from_launch(
                &config.args,
                config.session_mode == gwt_agent::SessionMode::Resume,
            );
            apply_windows_host_shell_wrapper(&mut config)?;

            let branch_name = config.branch.clone().unwrap_or_else(|| "work".to_string());

            let agent_id = config.agent_id.clone();
            let mut session =
                gwt_agent::Session::new(&worktree_path, branch_name.clone(), agent_id.clone());
            session.project_state_root = Some(
                gwt_core::paths::normalize_windows_child_process_path(Path::new(&project_root)),
            );
            session.display_name = config.display_name.clone();
            session.tool_version = config.tool_version.clone();
            session.model = config.model.clone();
            session.reasoning_level = config.reasoning_level.clone();
            session.session_mode = config.session_mode;
            session.skip_permissions = config.skip_permissions;
            session.fast_mode = config.fast_mode;
            session.codex_fast_mode = config.codex_fast_mode;
            session.runtime_target = config.runtime_target;
            session.docker_service = config.docker_service.clone();
            session.docker_lifecycle_intent = config.docker_lifecycle_intent;
            session.linked_issue_number = config.linked_issue_number;
            session.launch_command = config.command.clone();
            session.launch_args = config.args.clone();
            session.windows_shell = config.windows_shell;
            if session.session_mode == gwt_agent::SessionMode::Resume {
                session.agent_session_id = config.resume_session_id.clone();
            }
            session.update_status(gwt_agent::AgentStatus::Running);
            let prepared_continuation = match &config.execution_intent {
                gwt_agent::ExecutionLaunchIntent::Automatic => None,
                gwt_agent::ExecutionLaunchIntent::PreparedContinuation(binding) => {
                    if sessions_dir
                        .join(format!("{}.toml", binding.session_id))
                        .exists()
                    {
                        return Err(
                            "Prepared continuation Session already exists; reconcile the operation before retrying"
                                .to_string(),
                        );
                    }
                    session.id = binding.session_id.clone();
                    session.set_execution_binding(Some(binding.clone()))?;
                    Some(binding.clone())
                }
            };

            let session_id = session.id.clone();
            let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
            config.env_vars.insert(
                gwt_agent::GWT_SESSION_ID_ENV.to_string(),
                session_id.clone(),
            );
            // SPEC-3247 FR-001: export the session-kind signal into the spawned
            // agent's env HERE, in the production spawn path (the `prepare.rs`
            // helper is an alternate path with no production callers). Derived
            // from the same `config.is_ephemeral` as the materialization
            // guidance kind above, so the runtime signal and the materialized
            // guidance never disagree. Absent/unknown decodes to Execution
            // downstream (FR-004).
            let session_kind_env = gwt_skills::SessionKind::from_is_ephemeral(config.is_ephemeral)
                .as_env_str()
                .to_string();
            config.env_vars.insert(
                gwt_skills::GWT_SESSION_KIND_ENV.to_string(),
                session_kind_env,
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
            let docker_runtime_worktree = finalize_docker_agent_launch_config_with_runtime(
                Path::new(&project_root),
                &mut config,
                container_runtime.as_ref(),
            )?;
            let agent_project_root = docker_runtime_worktree.unwrap_or_else(|| {
                config
                    .env_vars
                    .get("GWT_PROJECT_ROOT")
                    .cloned()
                    .unwrap_or_else(|| worktree_path.display().to_string())
            });

            persist_finalized_launch_session(
                &sessions_dir,
                &runtime_path,
                &mut session,
                (runtime_target == gwt_agent::LaunchRuntimeTarget::Docker)
                    .then_some(agent_project_root.as_str()),
            )?;

            // A plain Resume is inspection-only. Producing authority is
            // created only for a linked non-ephemeral launch that owns its
            // execution lifecycle. Continue work creates successor
            // generations through its coordinator instead of falling through
            // this genesis-only launch path.
            let launch_disposition = launch_disposition(&config);
            let producing_owner = (!config.is_ephemeral
                && !config.suppress_execution_control
                && prepared_continuation.is_none()
                && launch_disposition == AgentLaunchDisposition::WorkProducing)
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
            FinalizedAgentCapabilityLaunch {
                issuer: agent_capability_issuer.as_ref(),
                sessions_dir: &sessions_dir,
                session: &mut session,
                project_root: Path::new(&project_root),
                worktree: &worktree_path,
                producing_owner,
                prepared_continuation: prepared_continuation.as_ref(),
                execution_entrypoint: &execution_entrypoint,
                runtime_target,
                container_runtime: container_runtime.as_ref(),
            }
            .install(&mut config.env_vars)?;
            issued_capability_token = config
                .env_vars
                .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
                .cloned();

            let process_launch = ProcessLaunch {
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env_vars.clone(),
                remove_env: config.remove_env.clone(),
                cwd: config.working_dir.clone(),
            };
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
                launch_disposition,
                agent_project_root,
            ))
        })();

        // Drop (= final drain + join) BEFORE dispatching the result so the
        // tail of the mirrored docker output lands in the terminal ahead of
        // the success transition or the `[gwt] Launch failed` summary —
        // otherwise the failure summary gets buried mid-stream.
        drop(docker_output_mirror);

        match result {
            Ok((
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
                linked_issue_number,
                base_branch,
                runtime_target,
                launch_disposition,
                agent_project_root,
            )) => {
                dispatch_agent_launch_success(
                    proxy,
                    window_id,
                    (
                        process_launch,
                        session_id,
                        branch_name,
                        display_name,
                        worktree_path,
                        agent_id,
                        linked_issue_number,
                        base_branch,
                        runtime_target,
                        launch_disposition,
                        agent_project_root,
                    ),
                    |proxy, project_index_root| {
                        crate::project_index_bootstrap::ProjectIndexBootstrapService::global()
                            .spawn(proxy, project_index_root);
                    },
                );
            }
            Err(error) => {
                if let (Some(issuer), Some(token)) = (
                    agent_capability_issuer.as_ref(),
                    issued_capability_token.as_deref(),
                ) {
                    issuer.revoke_token(token);
                }
                proxy.send(UserEvent::LaunchComplete {
                    window_id,
                    result: Err(error),
                });
            }
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
    pub(crate) fn close_work(&mut self, work_id: &str, close_kind: &str) -> Vec<OutboundEvent> {
        let work_id = work_id.trim();
        if work_id.is_empty() {
            return Vec::new();
        }
        let close_kind = match close_kind.trim().to_ascii_lowercase().as_str() {
            "done" => gwt_core::workspace_projection::WorkCloseKind::Done,
            "discarded" => gwt_core::workspace_projection::WorkCloseKind::Discarded,
            other => {
                tracing::warn!(
                    work_id = %work_id,
                    close_kind = %other,
                    "ignoring Work close with unknown close_kind"
                );
                return Vec::new();
            }
        };

        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            tracing::warn!(work_id = %work_id, "Work close has no active project tab");
            return Vec::new();
        };

        // The session id of an agent-session Work is encoded in the Work id
        // (`work-session-<session_id>`). A live agent owns the Work when any
        // active session matches that id.
        let session_id = work_id
            .strip_prefix("work-session-")
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let has_live_agent = session_id.is_some_and(|session_id| {
            self.active_agent_sessions
                .values()
                .any(|session| session.session_id == session_id)
        });

        let decision = gwt_core::workspace_projection::decide_work_close(has_live_agent, None);

        match decision {
            gwt_core::workspace_projection::WorkCloseDecision::BlockedLiveAgent => {
                // FR-352: never clean up a Work while its agent session is live.
                tracing::warn!(
                    work_id = %work_id,
                    session_id = session_id.unwrap_or_default(),
                    "Work close blocked: owning agent session is still live; stop the agent before closing"
                );
                return Vec::new();
            }
            gwt_core::workspace_projection::WorkCloseDecision::RecordOnly => {
                // Work close records lifecycle only. Worktree deletion is an
                // independent cleanup operation with its own safety gates.
            }
        }

        // Record the terminal close in the work history. Idempotent against an
        // already-closed Work, so a duplicate close emits no new event.
        let now = chrono::Utc::now();
        let recorded = match close_kind {
            gwt_core::workspace_projection::WorkCloseKind::Done => {
                gwt_core::workspace_projection::emit_workspace_done_event_if_absent(
                    &project_root,
                    work_id,
                    now,
                )
            }
            gwt_core::workspace_projection::WorkCloseKind::Discarded => {
                gwt_core::workspace_projection::emit_workspace_discard_event_if_absent(
                    &project_root,
                    work_id,
                    now,
                )
            }
        };
        if let Err(error) = recorded {
            tracing::warn!(
                work_id = %work_id,
                error = %error,
                "failed to record Work terminal close event"
            );
        }

        // Broadcast the refreshed projection so the Work leaves the active
        // surface for every connected client.
        self.active_work_projection_broadcast_for_active_tab()
            .into_iter()
            .collect()
    }

    pub(crate) fn mark_agent_session_stopped(&mut self, window_id: &str) {
        let inspection_only = self.inspection_agent_windows.remove(window_id);
        let Some(session) = self.active_agent_sessions.remove(window_id) else {
            self.revoke_agent_capability_for_window(window_id);
            return;
        };
        self.revoke_agent_capability_for_window(window_id);
        if inspection_only {
            let _ = gwt_agent::persist_session_status(
                &self.sessions_dir,
                &session.session_id,
                gwt_agent::AgentStatus::Stopped,
            );
            self.launch_wizard_cache.mark_stopped(&session.session_id);
            return;
        }
        // SPEC-3214 (FR-002 / T-005 / T-007): an ephemeral intake session runs
        // in a throwaway detached `.intake-*` worktree and produces NO Work
        // identity. On session end, remove the worktree when clean; keep it
        // when dirty so uncommitted work is never lost. Skip the Paused-Work /
        // projection persistence entirely.
        if self.is_ephemeral_intake_session(&session) {
            self.finalize_ephemeral_intake_worktree(&session);
            let _ = gwt_agent::persist_session_status(
                &self.sessions_dir,
                &session.session_id,
                gwt_agent::AgentStatus::Stopped,
            );
            self.launch_wizard_cache.mark_stopped(&session.session_id);
            return;
        }
        if let Some(project_root) = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
        {
            // SPEC-2359 Phase W-12 Slice 5a (FR-350): persist a Paused marker
            // before clearing the agent from the live projection so the Work is
            // retained on the Work surface until the user explicitly closes it.
            self.persist_paused_work_for_stopped_session(&project_root, &session);
            if let Err(error) = gwt_core::workspace_projection::mark_workspace_agent_stopped(
                &project_root,
                &session.session_id,
                Some(&session.window_id),
            ) {
                tracing::warn!(
                    error = %error,
                    project_root = %project_root.display(),
                    session_id = %session.session_id,
                    window_id = %session.window_id,
                    "failed to clean stopped Agent from Workspace projection"
                );
            }
        }
        let _ = gwt_agent::persist_session_status(
            &self.sessions_dir,
            &session.session_id,
            gwt_agent::AgentStatus::Stopped,
        );
        self.launch_wizard_cache.mark_stopped(&session.session_id);
    }

    /// SPEC-3214 (codex #3235 review): whether a stopped session is an
    /// ephemeral intake session. The `.intake-*` basename alone is not enough —
    /// a normal branch worktree a user happens to name `.intake-*` must keep its
    /// Paused-Work / resume behavior. The definitive signal is that the intake
    /// worktree is DETACHED (branchless), which only `create_detached` produces.
    /// A worktree that is already gone is treated as ephemeral (it was reaped).
    pub(super) fn is_ephemeral_intake_session(&self, session: &ActiveAgentSession) -> bool {
        if !is_ephemeral_intake_worktree(&session.worktree_path) {
            return false;
        }
        let Some(main_repo_path) = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
            .and_then(|root| gwt_git::worktree::main_worktree_root(&root).ok())
        else {
            return !session.worktree_path.exists();
        };
        match gwt_git::WorktreeManager::new(&main_repo_path).list() {
            Ok(worktrees) => worktrees
                .iter()
                .find(|info| same_worktree_path(&info.path, &session.worktree_path))
                // On a branch → a real worktree, not intake. Detached → intake.
                .is_none_or(|info| info.branch.is_none()),
            // Cannot enumerate: fall back to "gone means it was ephemeral".
            Err(_) => !session.worktree_path.exists(),
        }
    }

    fn revoke_unbound_agent_capability(&self, token: Option<&str>) {
        if let (Some(issuer), Some(token)) = (self.agent_capability_issuer.as_ref(), token) {
            issuer.revoke_token(token);
        }
    }

    pub(super) fn revoke_agent_capability_for_window(&mut self, window_id: &str) {
        let token = self.agent_capability_tokens.remove(window_id);
        self.revoke_unbound_agent_capability(token.as_deref());
    }

    /// SPEC-3214 (FR-002): tear down an ephemeral intake worktree when its
    /// session ends. A clean worktree is force-removed; a dirty one is kept and
    /// logged so uncommitted work is never destroyed (the user-facing retention
    /// notice ships with the intake UI in a later phase).
    fn finalize_ephemeral_intake_worktree(&self, session: &ActiveAgentSession) {
        let worktree_path = session.worktree_path.as_path();
        let main_repo_path = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
            .and_then(|root| gwt_git::worktree::main_worktree_root(&root).ok())
            .unwrap_or_else(|| worktree_path.to_path_buf());
        let manager = gwt_git::WorktreeManager::new(&main_repo_path);

        match manager.ephemeral_worktree_has_local_work_with(worktree_path, |entry| {
            intake_hook_config_is_disposable(worktree_path, entry)
        }) {
            Ok(true) => {
                tracing::warn!(
                    worktree_path = %worktree_path.display(),
                    "ephemeral intake worktree has local work (changes, ignored files, or commits); keeping it so nothing is lost"
                );
                return;
            }
            Ok(false) => {}
            Err(error) => {
                // Fail closed: if we cannot prove the worktree is empty, keep it.
                tracing::warn!(
                    worktree_path = %worktree_path.display(),
                    error = %error,
                    "could not determine intake worktree cleanliness; keeping it"
                );
                return;
            }
        }

        if let Err(error) = manager.remove_force(worktree_path) {
            tracing::warn!(
                worktree_path = %worktree_path.display(),
                error = %error,
                "failed to remove clean ephemeral intake worktree"
            );
        }
    }

    /// Compatibility hook for the runtime-status path. Current intake cleanup
    /// runs synchronously in `mark_agent_session_stopped()` after classifying
    /// the session by detached `.intake-*` worktree state, so there is no
    /// deferred queue to drain here.
    pub(crate) fn take_ephemeral_worktree_cleanup_events(&mut self) -> Vec<OutboundEvent> {
        Vec::new()
    }

    /// SPEC-2359 Phase W-12 Slice 5a (FR-350): record a Pause work event for a
    /// stopped agent session so the Work persists in the work history and keeps
    /// surfacing as Paused. The Work id is the session-derived canonical id
    /// (`work-session-<session_id>`) so a later resume groups the live agent onto
    /// the same row and dedupes the Paused entry away. Identity (title / branch /
    /// worktree / board refs) is recovered from the saved projection's matching
    /// agent and git details, falling back to the live session when unavailable.
    fn persist_paused_work_for_stopped_session(
        &self,
        project_root: &Path,
        session: &ActiveAgentSession,
    ) {
        let session_id = session.session_id.trim();
        if session_id.is_empty() {
            return;
        }
        let work_id = format!("work-session-{session_id}");
        let projection = gwt_core::workspace_projection::load_workspace_projection(project_root)
            .ok()
            .flatten();
        let agent_summary = projection
            .as_ref()
            .and_then(|projection| projection.latest_agent_for_session(session_id));
        // #3065: owner / summary / the title fallback must come from the
        // session's own Work item (resolved by branch container inside the
        // background thread below), never from the repo-shared projection —
        // its identity belongs to whatever Work last wrote it.
        let agent_title = agent_summary
            .and_then(|agent| {
                agent
                    .title_summary
                    .clone()
                    .or_else(|| agent.current_focus.clone())
            })
            .filter(|value| !value.trim().is_empty());
        let board_refs = projection
            .as_ref()
            .map(|projection| projection.board_refs.clone())
            .unwrap_or_default();
        let branch = agent_summary
            .and_then(|agent| agent.branch.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.branch.clone())
            })
            .or_else(|| Some(session.branch_name.clone()))
            .filter(|value| !value.trim().is_empty());
        let worktree_path = agent_summary
            .and_then(|agent| agent.worktree_path.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.worktree_path.clone())
            })
            .or_else(|| Some(session.worktree_path.clone()));
        let git_details = projection
            .as_ref()
            .and_then(|projection| projection.git_details.clone());
        let execution_container = (branch.is_some() || worktree_path.is_some()).then(|| {
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch,
                worktree_path,
                pr_number: git_details.as_ref().and_then(|details| details.pr_number),
                pr_url: git_details
                    .as_ref()
                    .and_then(|details| details.pr_url.clone()),
                pr_state: git_details
                    .as_ref()
                    .and_then(|details| details.pr_state.clone()),
            }
        });
        // Close-latency root fix (2026-06-12): the record loads + saves the
        // home works.json (megabytes once a project has hundreds of Works).
        // Doing that synchronously on the UI event loop made every agent
        // window × stall for seconds (sampled: serde to_vec_pretty dominating
        // the close handler). Inputs are gathered synchronously above from
        // the in-memory projection; the file IO runs on a background thread
        // and the workspace projection watcher broadcasts the refreshed rows
        // once the write lands.
        let project_root = project_root.to_path_buf();
        let session_id = session_id.to_string();
        let log_session_id = session.session_id.clone();
        let lookup_branch = execution_container
            .as_ref()
            .and_then(|container| container.branch.clone());
        let lookup_worktree = execution_container
            .as_ref()
            .and_then(|container| container.worktree_path.clone());
        let record = thread::spawn(move || {
            // #3065: resolve identity from the session's own Work item. The
            // works.json IO already happens on this background thread for the
            // record itself, so the lookup adds no UI-loop cost.
            let own_item = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
                .ok()
                .flatten()
                .and_then(|works| {
                    gwt_core::workspace_projection::find_work_item_for_container(
                        &works,
                        &project_root,
                        lookup_branch.as_deref(),
                        lookup_worktree.as_deref(),
                    )
                    .map(|item| {
                        (
                            item.title.clone(),
                            item.summary.clone().or_else(|| item.intent.clone()),
                            item.owner.clone(),
                        )
                    })
                });
            let (item_title, summary, owner) = own_item.unwrap_or((String::new(), None, None));
            let title =
                agent_title.or_else(|| Some(item_title).filter(|value| !value.trim().is_empty()));
            if let Err(error) = gwt_core::workspace_projection::record_workspace_work_paused_event(
                &project_root,
                &work_id,
                title.as_deref(),
                summary.as_deref(),
                owner.as_deref(),
                &board_refs,
                execution_container,
                Some(&session_id),
                chrono::Utc::now(),
            ) {
                tracing::warn!(
                    error = %error,
                    project_root = %project_root.display(),
                    session_id = %log_session_id,
                    work_id = %work_id,
                    "failed to persist Paused Work for stopped Agent session"
                );
            }
        });
        // Unit tests assert the projection immediately after a stop, so the
        // write is joined for determinism there; production detaches it.
        #[cfg(test)]
        let _ = record.join();
        #[cfg(not(test))]
        drop(record);
    }

    pub(crate) fn clear_agent_window_startup_restore(&self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.get(window_id) else {
            return;
        };
        let _ = gwt_agent::persist_session_restore_window_on_startup(
            &self.sessions_dir,
            &session.session_id,
            false,
        );
    }

    fn refresh_launch_wizard_session_cache(&mut self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.get(window_id) else {
            return;
        };
        let path = self
            .sessions_dir
            .join(format!("{}.toml", session.session_id));
        match gwt_agent::Session::load_and_migrate(&path) {
            Ok(session) => self.launch_wizard_cache.record_session(session),
            Err(error) => tracing::warn!(
                path = %path.display(),
                error = %error,
                "failed to refresh Launch Wizard session cache"
            ),
        }
    }
}

#[cfg(test)]
#[path = "agent_launch_stage_tests.rs"]
mod agent_launch_stage_tests;

#[cfg(test)]
mod docker_session_persistence_tests {
    use super::*;

    #[test]
    fn production_docker_session_reload_matches_finalized_process_worktree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let sessions_dir = temp.path().join("sessions");
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/production\n",
        )
        .expect("write compose");

        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .working_dir(&project)
            .branch("work/docker-production")
            .build();
        config.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        config.command = "codex".to_string();
        config.args = vec!["--no-alt-screen".to_string()];
        let runtime = crate::resolved_test_docker_runtime(temp.path());
        let runtime_worktree =
            finalize_docker_agent_launch_config_with_runtime(&project, &mut config, Some(&runtime))
                .expect("finalize Docker launch")
                .expect("Docker runtime worktree");

        let mut session = gwt_agent::Session::new(
            &project,
            "work/docker-production",
            gwt_agent::AgentId::Codex,
        );
        session.project_state_root = Some(project.clone());
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        let session_id = session.id.clone();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);

        persist_finalized_launch_session(
            &sessions_dir,
            &runtime_path,
            &mut session,
            Some(&runtime_worktree),
        )
        .expect("persist finalized production launch");

        let process_launch = ProcessLaunch {
            command: config.command,
            args: config.args,
            env: config.env_vars,
            remove_env: config.remove_env,
            cwd: config.working_dir,
        };
        assert_eq!(process_launch.command, runtime.binary());
        let workdir_index = process_launch
            .args
            .iter()
            .position(|arg| arg == "-w")
            .expect("compose exec -w");
        let process_worktree = process_launch
            .args
            .get(workdir_index + 1)
            .expect("compose exec worktree");
        let reloaded = gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
            .expect("reload production Session");
        let binding = reloaded
            .docker_runtime_binding
            .expect("persisted Docker binding");

        assert_eq!(
            binding.runtime_worktree_path,
            PathBuf::from(process_worktree)
        );
        assert_eq!(
            binding.project_state_scope_hash,
            gwt_core::paths::project_scope_hash(&project)
                .as_str()
                .to_string()
        );
        assert!(runtime_path.exists(), "runtime state must be persisted");
    }
}

#[cfg(test)]
mod agent_endpoint_env_tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn provider_continuity_is_inspection_unless_continue_work_prepares_execution() {
        let resume = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .session_mode(gwt_agent::SessionMode::Resume)
            .resume_session_id("conversation-existing")
            .build();
        let legacy_continue = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
            .session_mode(gwt_agent::SessionMode::Continue)
            .build();
        let normal = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex).build();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-continuation".to_string(),
            repo_hash: "repo-hash".to_string(),
            owner_kind: "spec".to_string(),
            owner_number: 2359,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-successor".to_string(),
                binding_id: "binding-operation".to_string(),
                ledger_head_hash: "head-operation".to_string(),
            },
            capability_generation: 2,
        };
        let prepared_resume = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .session_mode(gwt_agent::SessionMode::Resume)
            .resume_session_id("conversation-existing")
            .prepared_continuation(binding)
            .build();

        assert_eq!(
            launch_disposition(&resume),
            AgentLaunchDisposition::Inspection
        );
        assert_eq!(
            launch_disposition(&legacy_continue),
            AgentLaunchDisposition::Inspection
        );
        assert_eq!(
            launch_disposition(&normal),
            AgentLaunchDisposition::WorkProducing
        );
        assert_eq!(
            launch_disposition(&prepared_resume),
            AgentLaunchDisposition::WorkProducing
        );
    }

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &Path) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn init_execution_repo(repo: &Path) {
        std::fs::create_dir_all(repo).expect("create execution repository");
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "https://github.com/example/launch-binding.git",
            ],
        ] {
            let output = gwt_core::process::hidden_command("git")
                .args(&args)
                .current_dir(repo)
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    struct PersistedExecutionLaunch {
        project: PathBuf,
        sessions_dir: PathBuf,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        session: gwt_agent::Session,
    }

    fn persisted_execution_launch(home: &Path) -> PersistedExecutionLaunch {
        let project = home.join("project");
        init_execution_repo(&project);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            number: 2359,
        };
        let mut session =
            gwt_agent::Session::new(&project, "work/issue-2359", gwt_agent::AgentId::Codex);
        session.project_state_root = Some(project.clone());
        session.linked_issue_number = Some(owner.number);
        session.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
        persist_finalized_launch_session(&sessions_dir, &runtime_path, &mut session, None)
            .expect("persist finalized Session before authority");
        PersistedExecutionLaunch {
            project,
            sessions_dir,
            owner,
            session,
        }
    }

    #[test]
    fn producing_launch_persists_exact_binding_before_issuing_bound_capability() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut env)
        .expect("install exact bound launch authority");

        let persisted = gwt_agent::Session::load(
            &launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id)),
        )
        .expect("reload bound Session");
        let binding = persisted
            .execution_binding
            .expect("producing Session binding");
        assert_eq!(binding.capability_generation, 1);
        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read current generation")
                .expect("current generation binding"),
            binding.identity
        );
        let token = env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("bound capability token");
        let grant = issuer
            .grant_for_test(token)
            .expect("authenticate issued capability");
        assert!(grant.principal().authorizes_producing_mutation());
        assert_eq!(grant.principal().execution_binding(), Some(&binding));
    }

    #[test]
    fn prepared_continuation_issues_observation_only_capability_without_activating_successor() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut predecessor = persisted_execution_launch(home.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut predecessor_env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &predecessor.sessions_dir,
            session: &mut predecessor.session,
            project_root: &predecessor.project,
            worktree: &predecessor.project,
            producing_owner: Some(predecessor.owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut predecessor_env)
        .expect("materialize predecessor generation");
        assert!(matches!(
            gwt::cli::execution_state::settle(
                &predecessor.project,
                &predecessor.session.id,
                gwt::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle predecessor"),
            gwt::cli::execution_state::SettleResult::Settled(_)
        ));
        let settled_predecessor_identity = gwt::cli::execution_state::current_execution_binding(
            &predecessor.project,
            predecessor.owner,
        )
        .expect("read settled predecessor binding")
        .expect("settled predecessor generation");

        let continuation_session_id = uuid::Uuid::new_v4().to_string();
        let request = gwt::cli::execution_state::SuccessorRequest {
            operation_id: "prepared-launch-operation".to_string(),
            principal_id: "host-test-principal".to_string(),
            work_id: Some("work-prepared-launch".to_string()),
            source: "continue-work".to_string(),
            session_binding_id: uuid::Uuid::new_v4().to_string(),
            initial_session_id: continuation_session_id.clone(),
            entrypoint: "resume".to_string(),
            requested_at: chrono::Utc::now(),
        };
        gwt::cli::execution_state::prepare_successor(
            &predecessor.project,
            predecessor.owner,
            &request,
        )
        .expect("prepare successor");
        let planned_identity = gwt::cli::execution_state::prepared_successor_execution_binding(
            &predecessor.project,
            predecessor.owner,
            &request,
        )
        .expect("derive exact future binding");
        let mut continuation = gwt_agent::Session::new(
            &predecessor.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        continuation.id = continuation_session_id;
        continuation.project_state_root = Some(predecessor.project.clone());
        continuation.linked_issue_number = Some(predecessor.owner.number);
        continuation.update_status(gwt_agent::AgentStatus::Running);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: continuation.id.clone(),
            repo_hash: continuation
                .repo_hash
                .clone()
                .expect("continuation repository hash"),
            owner_kind: predecessor.owner.kind.as_str().to_string(),
            owner_number: predecessor.owner.number,
            identity: planned_identity.clone(),
            capability_generation: 1,
        };
        continuation
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Prepared continuation Session");
        continuation
            .save(&predecessor.sessions_dir)
            .expect("persist Prepared continuation Session");

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &predecessor.sessions_dir,
            session: &mut continuation,
            project_root: &predecessor.project,
            worktree: &predecessor.project,
            producing_owner: None,
            prepared_continuation: Some(&binding),
            execution_entrypoint: "resume",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut env)
        .expect("issue exact Prepared capability");

        let token = env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("Prepared capability token");
        let grant = issuer
            .grant_for_test(token)
            .expect("authenticate Prepared capability");
        assert_eq!(
            grant.principal().execution_authority_kind(),
            crate::embedded_server::AgentExecutionAuthorityKind::Prepared
        );
        assert!(!grant.principal().authorizes_producing_mutation());
        assert_eq!(grant.principal().execution_binding(), Some(&binding));
        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(
                &predecessor.project,
                predecessor.owner,
            )
            .expect("read current predecessor binding"),
            Some(settled_predecessor_identity),
            "issuing Prepared authority must not activate the successor"
        );
        assert!(
            gwt::cli::execution_state::prepared_execution_binding_matches(
                &predecessor.project,
                predecessor.owner,
                &continuation.id,
                &planned_identity,
            )
            .expect("read Prepared authority")
        );
    }

    #[test]
    fn explicit_linked_owner_launch_imports_generationless_blocked_predecessor_before_preparing_fresh_lifetime(
    ) {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let project = home.path().join("project");
        init_execution_repo(&project);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 1974,
        };
        let legacy_session_id = "fe3a92d8-60c6-46ae-8a83-1b3bac0891fa";
        gwt::cli::execution_state::materialize_at_launch(
            &project,
            owner.kind,
            owner.number,
            legacy_session_id,
            "$gwt-execute #1974",
            false,
        )
        .expect("materialize legacy flat execution");
        assert!(matches!(
            gwt::cli::execution_state::settle(
                &project,
                legacy_session_id,
                gwt::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "legacy delivery stopped before PR handoff".to_string(),
                    missing_verification: Some("legacy browser verification".to_string()),
                },
            )
            .expect("settle legacy flat execution"),
            gwt::cli::execution_state::SettleResult::Settled(_)
        ));
        assert!(
            gwt::cli::execution_state::current_execution_binding(&project, owner)
                .expect("probe generation-less legacy state")
                .is_none(),
            "the regression fixture must not pre-create a generation ledger"
        );
        let legacy_projection = gwt::cli::trusted_store::read(&project, "execution-control.json")
            .expect("read trusted legacy projection")
            .expect("trusted legacy projection");

        let mut successor =
            gwt_agent::Session::new(&project, "work/issue-1974", gwt_agent::AgentId::Codex);
        successor.project_state_root = Some(project.clone());
        successor.linked_issue_number = Some(owner.number);
        successor.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &successor.id);
        persist_finalized_launch_session(&sessions_dir, &runtime_path, &mut successor, None)
            .expect("persist fresh launch Session before authority");
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();

        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &sessions_dir,
            session: &mut successor,
            project_root: &project,
            worktree: &project,
            producing_owner: Some(owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #1974",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut env)
        .expect("prepare a fresh lifetime from the generation-less Blocked predecessor");

        assert_eq!(
            gwt::cli::trusted_store::read(&project, "execution-control.json")
                .expect("read imported predecessor projection")
                .expect("imported predecessor projection"),
            legacy_projection,
            "pre-readiness launch must preserve the exact terminal predecessor bytes"
        );
        let ledger = gwt::cli::execution_state::load_generation_ledger(&project, owner)
            .expect("read imported legacy ledger")
            .expect("imported legacy ledger");
        assert_eq!(ledger.generations.len(), 1);
        assert_eq!(
            ledger.generations[0].execution_control_json, legacy_projection,
            "legacy terminal bytes must remain the immutable predecessor snapshot"
        );
        assert_eq!(
            ledger.current_effective_status(),
            Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
        );
        assert_eq!(
            ledger.generations[0].identity.initial_session_id,
            legacy_session_id
        );
        let persisted =
            gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", successor.id)))
                .expect("reload fresh Prepared Session");
        let binding = persisted
            .execution_binding
            .expect("fresh launch must persist a Prepared successor binding");
        assert_ne!(
            gwt::cli::execution_state::current_execution_binding(&project, owner)
                .expect("read current predecessor binding")
                .expect("current predecessor binding"),
            binding.identity,
            "Prepared successor must not become current before authenticated Ready"
        );
        let attempt = gwt::cli::execution_state::prepared_fresh_linked_owner_launch_for_session(
            &project,
            owner,
            &successor.id,
        )
        .expect("read fresh Prepared attempt")
        .expect("fresh Prepared attempt");
        assert_eq!(
            attempt.status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
        );
        assert!(env.contains_key(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV));
        let token = env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("fresh Prepared capability token");
        let grant = issuer
            .grant_for_test(token)
            .expect("authenticate fresh Prepared capability");
        assert_eq!(
            grant.principal().execution_authority_kind(),
            crate::embedded_server::AgentExecutionAuthorityKind::Prepared
        );
    }

    #[test]
    fn explicit_linked_owner_launch_prepares_fresh_lifetime_from_blocked_predecessor() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut predecessor = persisted_execution_launch(home.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &predecessor.sessions_dir,
            session: &mut predecessor.session,
            project_root: &predecessor.project,
            worktree: &predecessor.project,
            producing_owner: Some(predecessor.owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut HashMap::new())
        .expect("materialize predecessor generation");
        assert!(matches!(
            gwt::cli::execution_state::settle(
                &predecessor.project,
                &predecessor.session.id,
                gwt::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "legacy blocker without same-lifetime recovery contract".to_string(),
                    missing_verification: Some("legacy blocker".to_string()),
                },
            )
            .expect("settle predecessor as Blocked"),
            gwt::cli::execution_state::SettleResult::Settled(_)
        ));
        let predecessor_binding = gwt::cli::execution_state::current_execution_binding(
            &predecessor.project,
            predecessor.owner,
        )
        .expect("read blocked predecessor binding")
        .expect("blocked predecessor generation");

        let mut successor = gwt_agent::Session::new(
            &predecessor.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        successor.project_state_root = Some(predecessor.project.clone());
        successor.linked_issue_number = Some(predecessor.owner.number);
        successor.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&predecessor.sessions_dir, &successor.id);
        persist_finalized_launch_session(
            &predecessor.sessions_dir,
            &runtime_path,
            &mut successor,
            None,
        )
        .expect("persist fresh launch Session before authority");

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &predecessor.sessions_dir,
            session: &mut successor,
            project_root: &predecessor.project,
            worktree: &predecessor.project,
            producing_owner: Some(predecessor.owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut env)
        .expect("explicit fresh launch must prepare a new lifetime from legacy Blocked");

        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(
                &predecessor.project,
                predecessor.owner,
            )
            .expect("read current generation"),
            Some(predecessor_binding),
            "pre-readiness preparation must preserve the terminal predecessor as current",
        );
        let persisted = gwt_agent::Session::load(
            &predecessor
                .sessions_dir
                .join(format!("{}.toml", successor.id)),
        )
        .expect("reload fresh prepared Session");
        let binding = persisted
            .execution_binding
            .expect("fresh launch must persist its prepared generation binding");
        let token = env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("fresh launch Prepared capability token");
        let grant = issuer
            .grant_for_test(token)
            .expect("authenticate fresh Prepared capability");
        assert_eq!(
            grant.principal().execution_authority_kind(),
            crate::embedded_server::AgentExecutionAuthorityKind::Prepared,
        );
        assert!(!grant.principal().authorizes_producing_mutation());
        assert_eq!(grant.principal().execution_binding(), Some(&binding));
    }

    #[test]
    fn failed_bound_capability_install_rolls_back_the_persisted_session_binding() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "not-a-hook-url",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let runtime = crate::resolved_test_docker_runtime(home.path());
        let mut env = HashMap::new();
        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
            container_runtime: Some(&runtime),
        }
        .install(&mut env)
        .expect_err("invalid Docker hook endpoint must fail");

        assert!(error.contains("invalid host hook forward URL"));
        let persisted = gwt_agent::Session::load(
            &launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id)),
        )
        .expect("reload failed launch Session");
        assert!(
            persisted.execution_binding.is_none(),
            "failed capability installation must not leave producing authority"
        );
        assert!(
            !env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "failed capability installation must not expose a bearer"
        );
        assert!(
            gwt::cli::execution_state::load(&launch.project)
                .expect("read failed launch ECR")
                .is_none(),
            "known endpoint failure must be rejected before flat ECR materialization"
        );
        assert!(
            gwt::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read failed launch generation")
                .is_none(),
            "known endpoint failure must not leave a ghost generation"
        );
    }

    #[test]
    fn producing_launch_without_host_issuer_refuses_before_execution_materialization() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let mut env = HashMap::new();
        let error = FinalizedAgentCapabilityLaunch {
            issuer: None,
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut env)
        .expect_err("producing launch requires its Host capability issuer");

        assert!(error.contains("Host capability issuer"));
        assert!(gwt::cli::execution_state::load(&launch.project)
            .expect("read refused launch ECR")
            .is_none());
        assert!(gwt::cli::execution_state::current_execution_binding(
            &launch.project,
            launch.owner
        )
        .expect("read refused launch generation")
        .is_none());
        let persisted = gwt_agent::Session::load(
            &launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id)),
        )
        .expect("reload refused launch Session");
        assert!(persisted.execution_binding.is_none());
    }

    #[test]
    fn producing_launch_preflights_a_closing_issuer_before_execution_materialization() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let inspection = issuer
            .issue(&launch.project, &launch.session.id)
            .expect("issue pre-existing inspection capability");
        let grant = issuer
            .grant_for_test(&inspection.token)
            .expect("authenticate pre-existing capability");
        let close_ticket = issuer
            .begin_self_close_if_current(&grant)
            .expect("hold issuer in closing state");
        let mut env = HashMap::new();
        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut env)
        .expect_err("closing Host issuer must refuse producing launch");

        assert!(issuer.rollback_self_close(&close_ticket));
        assert!(issuer.revoke_token(&inspection.token));
        assert!(error.contains("closing"));
        assert!(
            gwt::cli::execution_state::load(&launch.project)
                .expect("read closing launch ECR")
                .is_none(),
            "issuer availability must be rejected before flat ECR materialization"
        );
        assert!(
            gwt::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read closing launch generation")
                .is_none(),
            "issuer availability must be rejected before generation materialization"
        );
    }

    #[test]
    fn session_binding_io_failure_reports_the_materialized_generation_recovery_boundary() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let saved_sessions_dir = home.path().join("sessions-before-io-failure");
        std::fs::rename(&launch.sessions_dir, &saved_sessions_dir)
            .expect("move Session directory before injected failure");
        std::fs::write(&launch.sessions_dir, "not-a-directory")
            .expect("block Session directory recreation");
        let mut env = HashMap::new();
        let result = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
        }
        .install(&mut env);
        std::fs::remove_file(&launch.sessions_dir).expect("remove injected Session path blocker");
        std::fs::rename(&saved_sessions_dir, &launch.sessions_dir)
            .expect("restore Session directory after injected failure");
        let error = result.expect_err("Session binding persistence must fail");

        assert!(error.contains("execution generation was materialized"));
        assert!(error.contains("transactional recovery"));
        assert!(
            gwt::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read materialized generation")
                .is_some(),
            "the current ledger API has no safe genesis rollback seam yet"
        );
        let persisted = gwt_agent::Session::load(
            &launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id)),
        )
        .expect("reload pre-binding Session");
        assert!(persisted.execution_binding.is_none());
        assert!(
            !env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "binding persistence failure must precede bearer issuance"
        );
    }

    #[test]
    fn launch_injects_the_runtime_specific_pane_websocket_endpoint() {
        let project = tempfile::tempdir().expect("project tempdir");
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();

        install_agent_capability_env(
            &mut env,
            Some(&issuer),
            project.path(),
            "session-pane-env",
            gwt_agent::LaunchRuntimeTarget::Host,
            None,
        )
        .expect("install agent launch endpoints");

        assert_eq!(
            env.get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://127.0.0.1:45123/internal/hook-live")
        );
        assert_eq!(
            env.get(gwt_agent::GWT_PANE_WS_URL_ENV).map(String::as_str),
            Some("ws://127.0.0.1:45123/internal/pane-ws")
        );

        let mut docker_env = HashMap::new();
        let docker_runtime = crate::resolved_test_docker_runtime(project.path());
        install_agent_capability_env(
            &mut docker_env,
            Some(&issuer),
            project.path(),
            "session-pane-env-docker",
            gwt_agent::LaunchRuntimeTarget::Docker,
            Some(&docker_runtime),
        )
        .expect("install Docker agent launch endpoints");

        assert_eq!(
            docker_env
                .get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://host.docker.internal:45123/internal/hook-live")
        );
        assert_eq!(
            docker_env
                .get(gwt_agent::GWT_PANE_WS_URL_ENV)
                .map(String::as_str),
            Some("ws://host.docker.internal:45123/internal/pane-ws")
        );
    }

    #[cfg(unix)]
    #[test]
    fn launch_contract_pins_a_stateful_wrapper_across_override_and_endpoint_consumers() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let wrapper = temp.path().join("stateful-container-wrapper");
        std::fs::write(
            &wrapper,
            r#"#!/bin/sh
counter="$0.count"
count=0
if [ -f "$counter" ]; then
  read count < "$counter"
fi
count=$((count + 1))
printf '%s\n' "$count" > "$counter"
if [ "$count" -eq 1 ]; then
  printf 'Docker version 28.3.0, build test\n'
else
  printf 'podman version 5.4.2\n'
fi
"#,
        )
        .expect("write stateful wrapper");
        let mut permissions = std::fs::metadata(&wrapper)
            .expect("wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).expect("chmod stateful wrapper");

        let runtime = gwt_docker::detect::ResolvedContainerRuntime::resolve(
            wrapper.to_str().expect("UTF-8 wrapper path"),
        )
        .expect("resolve launch runtime once");
        let bundle = crate::docker_launch::docker_bundle_mounts_for_home(temp.path());
        let override_content = crate::docker_launch::docker_bundle_override_content_for_runtime(
            "app",
            &bundle,
            runtime.kind(),
        );
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();
        install_agent_capability_env(
            &mut env,
            Some(&issuer),
            temp.path(),
            "session-stateful-runtime",
            gwt_agent::LaunchRuntimeTarget::Docker,
            Some(&runtime),
        )
        .expect("install pinned Docker endpoints");

        assert!(override_content.contains(gwt_docker::DOCKER_HOST_GATEWAY_EXTRA_HOST));
        assert_eq!(
            env.get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://host.docker.internal:45123/internal/hook-live")
        );
        assert_eq!(
            env.get(gwt_agent::GWT_PANE_WS_URL_ENV).map(String::as_str),
            Some("ws://host.docker.internal:45123/internal/pane-ws")
        );
        assert_eq!(
            std::fs::read_to_string(wrapper.with_extension("count"))
                .expect("read wrapper probe count")
                .trim(),
            "1",
            "one launch must probe a configured runtime wrapper exactly once"
        );
    }
}

#[cfg(test)]
mod fr001_capability_cache_tests {
    use super::LaunchWizardMemoryCache;

    // SPEC-3170 FR-001: the Claude capability probes are resolved once at cache
    // load time and the getters return the stored booleans verbatim, so opening
    // the Launch wizard reads cached values instead of re-spawning
    // `claude --version` on the tao main event-loop thread.
    #[test]
    fn caches_claude_capabilities_for_reuse_without_reprobe() {
        let dir = tempfile::tempdir().expect("tempdir");

        let on = LaunchWizardMemoryCache::load_with_agent_options_and_capabilities(
            dir.path(),
            Vec::new(),
            true,
            true,
        );
        assert!(on.claude_ultracode_supported());
        assert!(on.claude_workflows_enabled());

        let off = LaunchWizardMemoryCache::load_with_agent_options_and_capabilities(
            dir.path(),
            Vec::new(),
            false,
            false,
        );
        assert!(!off.claude_ultracode_supported());
        assert!(!off.claude_workflows_enabled());
    }
}

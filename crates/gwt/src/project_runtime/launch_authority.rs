//! Launch capability installation shared by GUI and project runtime owners.

use std::collections::HashMap;
use std::path::Path;

use super::launch_payload::AgentLaunchError;
use crate::agent_capability::AgentCapabilityIssuer;

pub fn install_agent_capability_env(
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
            AgentCapabilityLaunchAuthority::Unbound,
            AgentCapabilityLaunchAuthority::Active,
        ),
        endpoints,
    )
}

pub struct PreflightedAgentCapabilityEndpoints {
    forward_url: String,
    pane_websocket_url: String,
}

pub fn preflight_agent_capability_endpoints(
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

pub fn issue_preflighted_agent_capability_env(
    env: &mut HashMap<String, String>,
    issuer: &AgentCapabilityIssuer,
    project_root: &Path,
    session_id: &str,
    execution_authority: AgentCapabilityLaunchAuthority<'_>,
    endpoints: PreflightedAgentCapabilityEndpoints,
) -> Result<(), String> {
    let target = match execution_authority {
        AgentCapabilityLaunchAuthority::Unbound => issuer.issue(project_root, session_id)?,
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
pub enum AgentCapabilityLaunchAuthority<'a> {
    Unbound,
    Prepared(&'a gwt_agent::SessionExecutionBinding),
    Active(&'a gwt_agent::SessionExecutionBinding),
}

/// Issue #3423 / Issue #4207: install a continuation whose generation is
/// already current and Active.
///
/// Both answers of the resume coordinator land here. `ReboundCurrent`
/// re-validated the predecessor Session against the generation it already
/// held; `SuccessorCreated` minted the next generation *and activated it* in
/// the same lease (`activate_successor_with_session_rebind`). Neither leaves a
/// Prepared attempt behind, so neither can be validated as one — the authority
/// they carry is re-proved against the live owner ledger instead.
pub struct ActiveContinuationInstall<'a> {
    pub issuer: Option<&'a AgentCapabilityIssuer>,
    pub sessions_dir: &'a Path,
    pub session: &'a gwt_agent::Session,
    pub project_root: &'a Path,
    pub worktree: &'a Path,
    pub owner: crate::cli::execution_state::ExecutionOwnerKey,
    pub binding: &'a gwt_agent::SessionExecutionBinding,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub container_runtime: Option<&'a gwt_docker::detect::ResolvedContainerRuntime>,
    /// How a refusal names this continuation, so the message keeps saying
    /// which launch shape was actually being installed.
    pub label: &'a str,
}

impl ActiveContinuationInstall<'_> {
    pub fn install(
        self,
        env: &mut HashMap<String, String>,
    ) -> Result<Option<gwt_agent::SessionActiveLaunchHandshake>, AgentLaunchError> {
        let Self {
            issuer,
            sessions_dir,
            session,
            project_root,
            worktree,
            owner,
            binding,
            runtime_target,
            container_runtime,
            label,
        } = self;
        if crate::cli::execution_state::current_execution_binding(worktree, owner)
            .map_err(|error| AgentLaunchError::authority_rejected(error.to_string()))?
            .as_ref()
            != Some(&binding.identity)
        {
            return Err(AgentLaunchError::authority_rejected(format!(
                "{label} no longer matches the current execution generation"
            )));
        }
        if session.execution_binding.as_ref() != Some(binding) {
            return Err(AgentLaunchError::authority_rejected(format!(
                "{label} Session binding changed before capability issuance"
            )));
        }
        let expected = gwt_agent::SessionExecutionIdentity::for_binding(session, binding)
            .map_err(AgentLaunchError::authority_rejected)?;
        let handshake = match crate::cli::execution_state::begin_active_session_launch_handshake_classified(
            sessions_dir,
            &expected,
        )
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                AgentLaunchError::active_conflict(error.to_string())
            } else {
                AgentLaunchError::authority_rejected(error.to_string())
            }
        })?
        {
            crate::cli::execution_state::ActiveSessionLaunchHandshakeOutcome::Acquired(handshake) => *handshake,
            crate::cli::execution_state::ActiveSessionLaunchHandshakeOutcome::Conflict => {
                return Err(AgentLaunchError::active_conflict(format!(
                    "{label} lost the exact Active launch handshake race"
                )));
            }
            crate::cli::execution_state::ActiveSessionLaunchHandshakeOutcome::AuthorityRejected => {
                return Err(AgentLaunchError::authority_rejected(format!(
                    "{label} no longer holds exact Active launch authority"
                )));
            }
        };
        if let Err(error) = install_agent_capability_env_with_binding(
            env,
            issuer,
            project_root,
            &session.id,
            Some(binding),
            runtime_target,
            container_runtime,
        ) {
            let _ = crate::cli::execution_state::finish_active_session_launch_handshake(
                sessions_dir,
                &handshake,
            );
            return Err(error.into());
        }
        Ok(Some(handshake))
    }
}

/// Issue #3625 / SPEC #3393 FR-026: install an authenticated Active resume
/// binding on the Session before capability issuance.
///
/// The continuation coordinator already activated and persisted this exact
/// binding for the owner it names. A Session that lost its linked owner is
/// repaired from the binding; a Session that names a different owner is left
/// untouched and the launch fails closed.
pub fn install_authenticated_active_resume_binding(
    session: &mut gwt_agent::Session,
    launch_linked_issue_number: &mut Option<u64>,
    binding: &gwt_agent::SessionExecutionBinding,
) -> Result<(), String> {
    if session
        .linked_issue_number
        .is_some_and(|owner_number| owner_number != binding.owner_number)
    {
        return Err(
            "Authenticated Resume owner does not match the linked Session owner".to_string(),
        );
    }

    session.id = binding.session_id.clone();
    session.linked_issue_number = Some(binding.owner_number);
    session.set_execution_binding(Some(binding.clone()))?;
    *launch_linked_issue_number = Some(binding.owner_number);
    Ok(())
}

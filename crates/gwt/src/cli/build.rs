//! `build.*` JSON lifecycle operations.
//!
//! Exit CLI for the `gwt-build-spec` skill (SPEC-1935 FR-014r). Writes
//! `.gwt/skill-state/build-spec.json` via [`gwt_core::skill_state`].

use gwt_github::SpecOpsError;

use super::skill_state_runtime;
use crate::cli::{CliEnv, SkillStateAction};

pub const SKILL_NAME: &str = "build-spec";
pub const SKILL_DISPLAY: &str = "gwt-build-spec";
pub const VERB: &str = "build";

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if let SkillStateAction::Start { spec } = &action {
        if !managed_build_start_preflight(env, *spec, out) {
            return Ok(2);
        }
    }
    if matches!(&action, SkillStateAction::Complete { .. }) {
        let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
        if let Some(refusal) =
            crate::cli::verification_record::work_event_settlement_refusal(&worktree)
        {
            out.push_str(&format!("{VERB}: completion refused — {refusal}\n"));
            return Ok(2);
        }
    }
    let recovered_orphan = match record_current_work_terminal_before_finalize(env, &action) {
        Ok(BuildTerminalizationDisposition::Proceed) => None,
        Ok(BuildTerminalizationDisposition::RecoverableOrphan { reason }) => Some(reason),
        Err(error) => {
            out.push_str(&format!("{VERB}: Work lifecycle update failed: {error}\n"));
            return Ok(1);
        }
    };
    // SPEC-3248 P8a: a successful build completion also settles the launch's
    // Execution Control Record (best-effort — the build-spec skill flow must
    // not require a second explicit `execution.complete`). Guarded strictly:
    // the settlement fires only when this `build.complete` actually finalized
    // an ACTIVE build state for the same spec — a vacuous "nothing to
    // finalize" exit 0 must not settle the execution — and only when the
    // record names the same owner. Aborting a build never settles.
    let completed_spec = match &action {
        SkillStateAction::Complete { spec } => {
            let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            let had_active_matching_state = gwt_core::skill_state::load(&worktree, SKILL_NAME)
                .ok()
                .flatten()
                .is_some_and(|state| {
                    state.active && (state.owner_spec.is_none() || state.owner_spec == Some(*spec))
                });
            had_active_matching_state.then_some(*spec)
        }
        _ => None,
    };
    let orphan_recovery_identity = recovered_orphan.as_ref().and_then(|_| match &action {
        SkillStateAction::Abort { spec, .. } => std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
            .ok()
            .map(|session_id| (*spec, session_id)),
        SkillStateAction::Start { .. }
        | SkillStateAction::Phase { .. }
        | SkillStateAction::Complete { .. } => None,
    });
    let code = skill_state_runtime::run(env, action, SKILL_NAME, SKILL_DISPLAY, VERB, out)?;
    if code == 0 {
        if let Some(reason) = recovered_orphan {
            let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            let Some((spec, session_id)) = orphan_recovery_identity else {
                out.push_str("build: orphan recovery identity was unavailable at readback\n");
                return Ok(1);
            };
            let recovered = gwt_core::skill_state::load(&worktree, SKILL_NAME)
                .ok()
                .flatten()
                .is_some_and(|state| {
                    !state.active
                        && state.owner_spec == Some(spec)
                        && state.session_id == session_id.trim()
                });
            if !recovered {
                out.push_str(
                    "build: orphan recovery could not be confirmed by lifecycle state readback\n",
                );
                return Ok(1);
            }
            out.push_str(&format!(
                "{VERB}: {}\n",
                serde_json::json!({
                    "ok": true,
                    "status": "orphan_recovered",
                    "reason": reason,
                })
            ));
        }
        if let Some(spec) = completed_spec {
            if let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
                // SPEC-3248 P8b (T-111): the execution settlement piggybacked
                // on build.complete also requires fresh verification
                // evidence; a build completion without it finalizes the
                // skill state but leaves the execution active so the Stop
                // gate keeps the session working toward real evidence.
                let has_matching_active_record = crate::cli::execution_state::load(&worktree)
                    .ok()
                    .flatten()
                    .is_some_and(|record| {
                        record.status == crate::cli::execution_state::ExecutionControlStatus::Active
                            && record.primary_session_id == session_id
                            && record.owner_number == spec
                    });
                if has_matching_active_record {
                    let status = crate::cli::verification_record::evaluate_evidence(
                        &worktree,
                        &session_id,
                        Some(spec),
                    );
                    if status == crate::cli::verification_record::EvidenceStatus::Fresh {
                        crate::cli::execution_state::settle_completed_best_effort(
                            &worktree,
                            &session_id,
                            spec,
                        );
                    } else {
                        out.push_str(&format!(
                            "{VERB}: execution control not settled — {}\n",
                            status.describe()
                        ));
                    }
                }
            }
        }
    }
    Ok(code)
}

fn managed_build_start_preflight<E: CliEnv>(env: &E, spec: u64, out: &mut String) -> bool {
    let current_session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .unwrap_or_default()
        .trim()
        .to_string();
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    match gwt_core::skill_state::load(&worktree, SKILL_NAME) {
        Ok(Some(state))
            if state.active && state.session_id.trim() != current_session_id.as_str() =>
        {
            push_start_preflight_refusal(
                out,
                "build_state_session_mismatch",
                "an active build lifecycle belongs to another Session",
                "session.relaunch",
            );
            return false;
        }
        Ok(Some(state))
            if state.active && state.owner_spec.is_none_or(|owner_spec| owner_spec != spec) =>
        {
            push_start_preflight_refusal(
                out,
                "build_state_owner_mismatch",
                "an active build lifecycle belongs to another or unknown owner",
                "build.abort",
            );
            return false;
        }
        Ok(Some(_)) | Ok(None) => {}
        Err(_) => {
            push_start_preflight_refusal(
                out,
                "build_state_invalid",
                "the existing build lifecycle state is unreadable",
                "build.inspect",
            );
            return false;
        }
    }
    if !managed_build_context_present(env) {
        return true;
    }
    if current_session_id.is_empty() {
        push_start_preflight_refusal(
            out,
            "relaunch_required",
            "managed build.start is missing its Session identity",
            "session.relaunch",
        );
        return false;
    }
    if let Ok(Some(record)) = crate::cli::execution_state::load(&worktree) {
        if record.primary_session_id != current_session_id {
            push_start_preflight_refusal(
                out,
                "execution_session_mismatch",
                "current Session does not own the worktree Execution",
                "session.relaunch",
            );
            return false;
        }
        if record.owner_number != spec {
            push_start_preflight_refusal(
                out,
                "execution_owner_mismatch",
                "requested build owner does not match the current Execution owner",
                "build.start",
            );
            return false;
        }
    }
    let target = match crate::daemon_runtime::HookForwardTarget::from_env_strict() {
        Ok(Some(target)) => target,
        Ok(None) => {
            push_start_preflight_refusal(
                out,
                "relaunch_required",
                "managed build.start is missing its Host bridge capability",
                "session.relaunch",
            );
            return false;
        }
        Err(_) => {
            push_start_preflight_refusal(
                out,
                "relaunch_required",
                "managed build.start has an incomplete or invalid Host bridge capability",
                "session.relaunch",
            );
            return false;
        }
    };
    let observation = match crate::observe_agent_runtime(env.repo_path()) {
        Ok(observation) => observation,
        Err(error) => {
            push_start_preflight_refusal(
                out,
                workspace_error_code(error.code),
                preflight_rejection_reason(error.code),
                recovery_operation(error.code),
            );
            return false;
        }
    };
    let request = crate::AgentWorkMaterializationProbeRequest {
        schema_version: crate::AGENT_WORK_MATERIALIZATION_PROBE_SCHEMA_VERSION,
        claimed_session_id: current_session_id,
        owner_number: spec,
        observation,
    };
    match crate::daemon_runtime::send_work_materialization_probe_via_agent_bridge(&target, &request)
    {
        Ok(_) => true,
        Err(crate::daemon_runtime::AgentBridgeRequestError::Rejected(error)) => {
            push_start_preflight_refusal(
                out,
                workspace_error_code(error.code),
                preflight_rejection_reason(error.code),
                recovery_operation(error.code),
            );
            false
        }
        Err(error) => {
            push_start_preflight_refusal(
                out,
                "host_outcome_unknown",
                &error.to_string(),
                "session.relaunch",
            );
            false
        }
    }
}

fn push_start_preflight_refusal(
    out: &mut String,
    error_code: &str,
    reason: &str,
    recovery_operation: &str,
) {
    out.push_str(&format!(
        "{}\n",
        serde_json::json!({
            "ok": false,
            "error_code": error_code,
            "reason": reason,
            "recovery_operation": recovery_operation,
            "retryable": true,
        })
    ));
}

fn workspace_error_code(code: crate::AgentWorkspaceUpdateErrorCode) -> &'static str {
    match code {
        crate::AgentWorkspaceUpdateErrorCode::InvalidRequest => "invalid_request",
        crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired => "relaunch_required",
        crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => {
            "execution_binding_mismatch"
        }
        crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired => {
            "workspace_ensure_required"
        }
        crate::AgentWorkspaceUpdateErrorCode::ProvenanceMismatch => "provenance_mismatch",
        crate::AgentWorkspaceUpdateErrorCode::IdentityConflict => "identity_conflict",
        crate::AgentWorkspaceUpdateErrorCode::TransactionConflict => "transaction_conflict",
        crate::AgentWorkspaceUpdateErrorCode::Internal => "internal",
    }
}

fn recovery_operation(code: crate::AgentWorkspaceUpdateErrorCode) -> &'static str {
    match code {
        crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired => "workspace.ensure",
        crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired
        | crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => "session.relaunch",
        crate::AgentWorkspaceUpdateErrorCode::InvalidRequest
        | crate::AgentWorkspaceUpdateErrorCode::ProvenanceMismatch
        | crate::AgentWorkspaceUpdateErrorCode::IdentityConflict
        | crate::AgentWorkspaceUpdateErrorCode::TransactionConflict
        | crate::AgentWorkspaceUpdateErrorCode::Internal => "host.inspect",
    }
}

fn preflight_rejection_reason(code: crate::AgentWorkspaceUpdateErrorCode) -> &'static str {
    match code {
        crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired => {
            "Session-bound Work is not materialized"
        }
        crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired
        | crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => {
            "managed Session authority is missing, stale, or no longer current"
        }
        crate::AgentWorkspaceUpdateErrorCode::InvalidRequest => {
            "managed build.start preflight request or runtime observation is invalid"
        }
        crate::AgentWorkspaceUpdateErrorCode::ProvenanceMismatch => {
            "runtime provenance does not match the authenticated Session"
        }
        crate::AgentWorkspaceUpdateErrorCode::IdentityConflict => {
            "Session-bound Work identity conflicts with the current authority"
        }
        crate::AgentWorkspaceUpdateErrorCode::TransactionConflict => {
            "Host workspace authority changed during preflight"
        }
        crate::AgentWorkspaceUpdateErrorCode::Internal => {
            "Host could not determine the Session-bound Work"
        }
    }
}

fn managed_build_context_present<E: CliEnv>(env: &E) -> bool {
    if std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).is_some()
        || std::env::var_os(gwt_agent::GWT_HOOK_FORWARD_URL_ENV).is_some()
        || std::env::var_os(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV).is_some()
    {
        return true;
    }
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    match crate::cli::execution_state::load(&worktree) {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => crate::cli::execution_state::state_path(&worktree).exists(),
    }
}

fn record_current_work_terminal_before_finalize<E: CliEnv>(
    env: &E,
    action: &SkillStateAction,
) -> Result<BuildTerminalizationDisposition, String> {
    let (spec, close_kind) = match action {
        SkillStateAction::Complete { spec } => (*spec, WorkTerminalKind::Done),
        SkillStateAction::Abort { spec, .. } => (*spec, WorkTerminalKind::Discarded),
        SkillStateAction::Start { .. } | SkillStateAction::Phase { .. } => {
            return Ok(BuildTerminalizationDisposition::Proceed);
        }
    };
    let repo = env.repo_path();
    let state = gwt_core::skill_state::load(repo, SKILL_NAME).map_err(|error| error.to_string())?;
    let Some(state) = state else {
        return Ok(BuildTerminalizationDisposition::Proceed);
    };
    if state.owner_spec.is_some() && state.owner_spec != Some(spec) {
        return Ok(BuildTerminalizationDisposition::Proceed);
    }
    if !state.active {
        return Ok(BuildTerminalizationDisposition::Proceed);
    }
    let allow_orphan_recovery =
        close_kind == WorkTerminalKind::Discarded && state.owner_spec == Some(spec);

    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .unwrap_or_default()
        .trim()
        .to_string();
    if session_id.is_empty() {
        if state.session_id.trim().is_empty() && !managed_build_context_present(env) {
            return Ok(BuildTerminalizationDisposition::Proceed);
        }
        return Err(
            "active build state has no current Session authority; relaunch the owning Session before finalizing"
                .to_string(),
        );
    }
    if state.session_id.trim() != session_id {
        return Err(
            "active build state belongs to another Session; only its owning Session may finalize it"
                .to_string(),
        );
    }

    let bridge_target = match crate::daemon_runtime::HookForwardTarget::from_env_strict() {
        Ok(target) => target,
        Err(error) if allow_orphan_recovery => {
            return Ok(BuildTerminalizationDisposition::RecoverableOrphan {
                reason: format!("Host bridge request was not sent: {error}"),
            });
        }
        Err(error) => return Err(error),
    };
    if let Some(target) = bridge_target {
        let observation = crate::observe_agent_runtime(repo).map_err(|error| error.to_string())?;
        let request = crate::AgentWorkTerminalizationRequest {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            claimed_session_id: session_id,
            owner_number: spec,
            observation,
            terminal_kind: match close_kind {
                WorkTerminalKind::Done => crate::AgentWorkTerminalKind::Done,
                WorkTerminalKind::Discarded => crate::AgentWorkTerminalKind::Discarded,
            },
        };
        return match crate::daemon_runtime::send_work_terminalization_via_agent_bridge(
            &target, &request,
        ) {
            Ok(receipt) => {
                map_agent_terminal_outcome(receipt.outcome, close_kind, allow_orphan_recovery)
            }
            Err(error) if allow_orphan_recovery && error.proves_zero_mutation() => {
                Ok(BuildTerminalizationDisposition::RecoverableOrphan {
                    reason: error.to_string(),
                })
            }
            Err(error) => Err(error.to_string()),
        };
    }
    if std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).is_some() {
        if allow_orphan_recovery {
            return Ok(BuildTerminalizationDisposition::RecoverableOrphan {
                reason:
                    "managed Host bridge capability was absent, so no Work mutation was attempted"
                        .to_string(),
            });
        }
        return Err(
            "managed build terminalization is missing its Host bridge capability; relaunch the Session"
                .to_string(),
        );
    }

    let (project_state_root, work_event_root) =
        crate::agent_project_state::agent_session_roots_or_fallback(repo, &session_id)
            .map_err(|error| error.to_string())?;
    let legacy_work_id = format!("work-session-{session_id}");

    let now = chrono::Utc::now();
    let outcome = match close_kind {
        WorkTerminalKind::Done => {
            gwt_core::workspace_projection::emit_workspace_done_event_for_session_outcome(
                &project_state_root,
                &work_event_root,
                &session_id,
                &legacy_work_id,
                now,
            )
        }
        WorkTerminalKind::Discarded => {
            gwt_core::workspace_projection::emit_workspace_discard_event_for_session_outcome(
                &project_state_root,
                &work_event_root,
                &session_id,
                &legacy_work_id,
                now,
            )
        }
    }
    .map_err(|error| error.to_string())?;
    match outcome {
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::Emitted
        | gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AlreadyMatching
        | gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::NoTarget => {
            Ok(BuildTerminalizationDisposition::Proceed)
        }
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AssignedWorkMissing(
            work_id,
        ) if allow_orphan_recovery => {
            Ok(BuildTerminalizationDisposition::RecoverableOrphan {
                reason: format!(
                    "assigned Work {work_id} is not materialized and no terminal event was emitted"
                ),
            })
        }
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AssignedWorkMissing(
            work_id,
        ) => Err(format!(
            "assigned Work {work_id} is not materialized; retry workspace.ensure before finalizing the build"
        )),
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::WrongTerminal => Err(
            format!(
                "assigned Work has the wrong terminal state for {}",
                close_kind.as_str()
            ),
        ),
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AmbiguousTerminal => Err(
            "assigned Work has ambiguous Done and Discarded terminal state".to_string(),
        ),
    }
}

fn map_agent_terminal_outcome(
    outcome: crate::AgentWorkTerminalizationOutcome,
    close_kind: WorkTerminalKind,
    allow_orphan_recovery: bool,
) -> Result<BuildTerminalizationDisposition, String> {
    match outcome {
        crate::AgentWorkTerminalizationOutcome::Emitted
        | crate::AgentWorkTerminalizationOutcome::AlreadyMatching
        | crate::AgentWorkTerminalizationOutcome::NoTarget => {
            Ok(BuildTerminalizationDisposition::Proceed)
        }
        crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing if allow_orphan_recovery => {
            Ok(BuildTerminalizationDisposition::RecoverableOrphan {
                reason:
                    "Host confirmed the assigned Work is absent and emitted no terminal mutation"
                        .to_string(),
            })
        }
        crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing => Err(
            "assigned Work is not materialized; retry workspace.ensure before finalizing the build"
                .to_string(),
        ),
        crate::AgentWorkTerminalizationOutcome::WrongTerminal => Err(format!(
            "assigned Work has the wrong terminal state for {}",
            close_kind.as_str()
        )),
        crate::AgentWorkTerminalizationOutcome::AmbiguousTerminal => {
            Err("assigned Work has ambiguous Done and Discarded terminal state".to_string())
        }
    }
}

#[derive(Debug)]
enum BuildTerminalizationDisposition {
    Proceed,
    RecoverableOrphan { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkTerminalKind {
    Done,
    Discarded,
}

impl WorkTerminalKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Done => "Done",
            Self::Discarded => "Discarded",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };
    use gwt_core::test_support::ScopedEnvVar;
    use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

    use super::*;

    struct TerminalBridgeServer {
        runtime: Runtime,
        shutdown_tx: Option<oneshot::Sender<()>>,
        rx: mpsc::Receiver<(HeaderMap, serde_json::Value)>,
        forward_url: String,
    }

    #[derive(Clone)]
    struct TerminalBridgeState {
        tx: mpsc::Sender<(HeaderMap, serde_json::Value)>,
        status: StatusCode,
        body: String,
    }

    impl TerminalBridgeServer {
        fn start(status: StatusCode, body: serde_json::Value) -> Self {
            let runtime = Runtime::new().expect("terminal bridge runtime");
            let listener = runtime
                .block_on(TcpListener::bind(("127.0.0.1", 0)))
                .expect("terminal bridge listener");
            let address = listener.local_addr().expect("terminal bridge address");
            let (tx, rx) = mpsc::channel();
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let app = Router::new()
                .route(
                    "/internal/work-materialization-probe",
                    post(
                        |headers: HeaderMap,
                         State(state): State<TerminalBridgeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .tx
                                .send((headers, body))
                                .expect("capture materialization probe request");
                            (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response()
                        },
                    ),
                )
                .route(
                    "/internal/work-terminalization",
                    post(
                        |headers: HeaderMap,
                         State(state): State<TerminalBridgeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .tx
                                .send((headers, body))
                                .expect("capture terminal bridge request");
                            (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response()
                        },
                    ),
                )
                .with_state(TerminalBridgeState {
                    tx,
                    status,
                    body: body.to_string(),
                });
            runtime.spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .expect("terminal bridge server");
            });
            Self {
                runtime,
                shutdown_tx: Some(shutdown_tx),
                rx,
                forward_url: format!("http://127.0.0.1:{}/internal/hook-live", address.port()),
            }
        }

        fn receive(&self) -> (HeaderMap, serde_json::Value) {
            self.rx
                .recv_timeout(Duration::from_secs(2))
                .expect("terminal bridge request")
        }

        fn assert_no_request(&self) {
            assert!(
                matches!(self.rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "rejected lifecycle ownership must not reach the Host bridge"
            );
        }
    }

    impl Drop for TerminalBridgeServer {
        fn drop(&mut self) {
            if let Some(shutdown_tx) = self.shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
            self.runtime
                .block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
        }
    }

    fn terminal_receipt(outcome: crate::AgentWorkTerminalizationOutcome) -> serde_json::Value {
        terminal_receipt_for_owner(outcome, 3327)
    }

    fn terminal_receipt_for_owner(
        outcome: crate::AgentWorkTerminalizationOutcome,
        owner_number: u64,
    ) -> serde_json::Value {
        serde_json::to_value(crate::AgentWorkTerminalizationReceipt {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            owner_number,
            outcome,
        })
        .expect("serialize terminal receipt")
    }

    fn run_active_action(
        action: SkillStateAction,
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        run_active_action_with_owner(action, forward_url, forward_token, managed, Some(3327))
    }

    fn run_active_action_with_owner(
        action: SkillStateAction,
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
        owner_spec: Option<u64>,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        run_active_action_with_identity(
            action,
            forward_url,
            forward_token,
            managed,
            owner_spec,
            "terminal-bridge-session",
            Some("terminal-bridge-session"),
        )
    }

    fn run_active_action_with_identity(
        action: SkillStateAction,
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
        owner_spec: Option<u64>,
        state_session_id: &str,
        current_session_id: Option<&str>,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        gwt_core::skill_state::save(
            &fixture.repo,
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                active: true,
                owner_spec,
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: state_session_id.to_string(),
            },
        )
        .expect("save active build state");
        let _session = current_session_id.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, value),
        );
        let _forward_url = forward_url.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, value),
        );
        let _forward_token = forward_token.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, value),
        );
        let _runtime = if managed {
            ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
                fixture.repo.join("managed-runtime.json"),
            )
        } else {
            ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        };
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut output = String::new();
        let code = run(&mut env, action, &mut output).expect("run build action");
        (code, output, fixture)
    }

    fn run_start_action(
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        run_start_action_with_execution_record(forward_url, forward_token, managed, None, 3403)
    }

    fn run_start_action_with_execution_record(
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
        execution_session_id: Option<&str>,
        requested_spec: u64,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        if let Some(execution_session_id) = execution_session_id {
            crate::cli::execution_state::save(
                &fixture.repo,
                &crate::cli::execution_state::ExecutionControlRecord {
                    owner_kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    owner_number: 3403,
                    primary_session_id: execution_session_id.to_string(),
                    entrypoint: "$gwt-execute".to_string(),
                    bundled_required_owners: Vec::new(),
                    status: crate::cli::execution_state::ExecutionControlStatus::Active,
                    blocked_reason: None,
                    missing_verification: None,
                    launched_at: chrono::Utc::now(),
                    settled_at: None,
                    transfers: Vec::new(),
                    recoveries: Vec::new(),
                    content_hash: String::new(),
                },
            )
            .expect("save active execution record");
        }
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "start-bridge-session");
        let _forward_url = forward_url.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, value),
        );
        let _forward_token = forward_token.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, value),
        );
        let _runtime = if managed {
            ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
                fixture.repo.join("managed-runtime.json"),
            )
        } else {
            ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        };
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut output = String::new();
        let code = run(
            &mut env,
            SkillStateAction::Start {
                spec: requested_spec,
            },
            &mut output,
        )
        .expect("run build start");
        (code, output, fixture)
    }

    fn assert_build_still_active(
        fixture: &crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load build state")
                .expect("build state")
                .active,
            "failed Host terminalization must not finalize build state"
        );
    }

    #[test]
    fn managed_build_terminalization_uses_host_outcome_without_local_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let emitted = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
        );
        let (code, output, fixture) = run_active_action(
            SkillStateAction::Complete { spec: 3327 },
            Some(&emitted.forward_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 0, "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load build state")
                .expect("build state")
                .active,
            "pre-gated Complete may finalize in the same call after Host emission"
        );
        let (headers, request) = emitted.receive();
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer terminal-secret")
        );
        assert_eq!(request["claimed_session_id"], "terminal-bridge-session");
        assert_eq!(request["owner_number"], 3327);
        assert_eq!(request["terminal_kind"], "done");
        assert!(request.get("work_id").is_none());
        assert!(request.get("project_root").is_none());

        let retried = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let (code, output, fixture) = run_active_action(
            SkillStateAction::Complete { spec: 3327 },
            Some(&retried.forward_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 0, "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load build state")
                .expect("build state")
                .active,
            "idempotent Host retry must allow build finalization"
        );

        let unassigned = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
        );
        let (code, output, fixture) = run_active_action(
            SkillStateAction::Complete { spec: 3327 },
            Some(&unassigned.forward_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 0, "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load unassigned build state")
                .expect("unassigned build state")
                .active,
            "latest Unassigned is a safe idempotent no-op"
        );

        let discarded = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
        );
        let (code, output, fixture) = run_active_action(
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("cancelled".to_string()),
            },
            Some(&discarded.forward_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 0, "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load abort state")
                .expect("abort state")
                .active,
            "Abort may finalize in the same call after Host emission"
        );
        let (_, request) = discarded.receive();
        assert_eq!(request["terminal_kind"], "discarded");

        let discarded_retry = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let (code, output, _) = run_active_action(
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("cancelled".to_string()),
            },
            Some(&discarded_retry.forward_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 0, "{output}");
        let (_, request) = discarded_retry.receive();
        assert_eq!(request["terminal_kind"], "discarded");
    }

    #[test]
    fn managed_build_terminalization_failures_never_finalize_or_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, url, token) in [
            (
                "url-only",
                Some("http://127.0.0.1:45123/internal/hook-live"),
                None,
            ),
            ("token-only", None, Some("terminal-secret")),
            ("managed-missing", None, None),
        ] {
            let (code, output, fixture) =
                run_active_action(SkillStateAction::Complete { spec: 3327 }, url, token, true);
            assert_eq!(code, 1, "{label}: {output}");
            assert_build_still_active(&fixture);
        }

        for (label, status, body) in [
            (
                "authentication",
                StatusCode::UNAUTHORIZED,
                serde_json::json!({
                    "code": "invalid_request",
                    "message": "untrusted Host diagnostic terminal-secret"
                }),
            ),
            (
                "invalid-response",
                StatusCode::OK,
                serde_json::json!({
                    "schema_version": crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION + 1,
                    "owner_number": 3327,
                    "outcome": "already_matching"
                }),
            ),
            (
                "wrong-terminal",
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::WrongTerminal),
            ),
            (
                "wrong-owner-receipt",
                StatusCode::OK,
                terminal_receipt_for_owner(
                    crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing,
                    2359,
                ),
            ),
            (
                "assigned-work-missing",
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing),
            ),
            (
                "ambiguous-terminal",
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::AmbiguousTerminal),
            ),
        ] {
            let server = TerminalBridgeServer::start(status, body);
            let (code, output, fixture) = run_active_action(
                SkillStateAction::Complete { spec: 3327 },
                Some(&server.forward_url),
                Some("terminal-secret"),
                true,
            );
            assert_eq!(code, 1, "{label}: {output}");
            assert!(
                !output.contains("terminal-secret"),
                "{label}: Host response must not reflect the bearer into diagnostics: {output}"
            );
            assert_build_still_active(&fixture);
            server.receive();
        }

        let unavailable = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve unavailable bridge address");
        let port = unavailable
            .local_addr()
            .expect("unavailable bridge address")
            .port();
        drop(unavailable);
        let unavailable_url = format!("http://127.0.0.1:{port}/internal/hook-live");
        let (code, output, fixture) = run_active_action(
            SkillStateAction::Complete { spec: 3327 },
            Some(&unavailable_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 1, "transport: {output}");
        assert_build_still_active(&fixture);
    }

    #[test]
    fn managed_build_start_requires_materialized_work_before_creating_state() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let rejected = TerminalBridgeServer::start(
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "message": "untrusted Host diagnostic terminal-secret"
            }),
        );
        let (code, output, fixture) =
            run_start_action(Some(&rejected.forward_url), Some("terminal-secret"), true);
        assert_eq!(code, 2, "{output}");
        let refusal: serde_json::Value =
            serde_json::from_str(output.trim()).expect("typed build.start refusal");
        assert_eq!(refusal["ok"], false);
        assert_eq!(refusal["error_code"], "workspace_ensure_required");
        assert_eq!(refusal["recovery_operation"], "workspace.ensure");
        assert!(refusal["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()));
        assert!(
            !output.contains("terminal-secret"),
            "Host response must not reflect the bearer into diagnostics: {output}"
        );
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load rejected build state")
                .is_none(),
            "managed rejection must not create lifecycle state"
        );
        let (headers, request) = rejected.receive();
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer terminal-secret")
        );
        assert_eq!(request["claimed_session_id"], "start-bridge-session");
        assert_eq!(request["owner_number"], 3403);
        assert!(request.get("work_id").is_none());

        let accepted = TerminalBridgeServer::start(
            StatusCode::OK,
            serde_json::json!({
                "schema_version": 1,
                "owner_number": 3403,
                "work_id": "work-feature-build-start"
            }),
        );
        let (code, output, fixture) =
            run_start_action(Some(&accepted.forward_url), Some("terminal-secret"), true);
        assert_eq!(code, 0, "{output}");
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load accepted build state")
                .expect("accepted build state")
                .active,
            "successful managed preflight may create lifecycle state"
        );
        accepted.receive();
    }

    #[test]
    fn managed_build_start_rejects_incomplete_bridge_without_creating_state() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, url, token) in [
            (
                "url-only",
                Some("http://127.0.0.1:45123/internal/hook-live"),
                None,
            ),
            ("token-only", None, Some("terminal-secret")),
            ("managed-missing", None, None),
        ] {
            let (code, output, fixture) = run_start_action(url, token, true);
            assert_eq!(code, 2, "{label}: {output}");
            let refusal: serde_json::Value = serde_json::from_str(output.trim())
                .unwrap_or_else(|error| panic!("{label}: typed refusal: {error}: {output}"));
            assert_eq!(
                refusal["error_code"], "relaunch_required",
                "{label}: {output}"
            );
            assert_eq!(
                refusal["recovery_operation"], "session.relaunch",
                "{label}: {output}"
            );
            assert!(
                gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                    .expect("load rejected build state")
                    .is_none(),
                "{label}: missing managed capability must not create lifecycle state"
            );
        }
    }

    #[test]
    fn managed_build_start_rejects_invalid_probe_receipts_without_creating_state() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, body) in [
            (
                "invalid-schema",
                serde_json::json!({
                    "schema_version": crate::AGENT_WORK_MATERIALIZATION_PROBE_SCHEMA_VERSION + 1,
                    "owner_number": 3403,
                    "work_id": "work-feature-build-start"
                }),
            ),
            (
                "empty-work-id",
                serde_json::json!({
                    "schema_version": 1,
                    "owner_number": 3403,
                    "work_id": ""
                }),
            ),
            (
                "wrong-owner",
                serde_json::json!({
                    "schema_version": 1,
                    "owner_number": 2359,
                    "work_id": "work-feature-build-start"
                }),
            ),
        ] {
            let server = TerminalBridgeServer::start(StatusCode::OK, body);
            let (code, output, fixture) =
                run_start_action(Some(&server.forward_url), Some("terminal-secret"), true);
            assert_eq!(code, 2, "{label}: {output}");
            let refusal: serde_json::Value = serde_json::from_str(output.trim())
                .unwrap_or_else(|error| panic!("{label}: typed refusal: {error}: {output}"));
            assert_eq!(refusal["error_code"], "host_outcome_unknown");
            assert!(
                gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                    .expect("load rejected build state")
                    .is_none(),
                "{label}: invalid receipt must not create lifecycle state"
            );
            server.receive();
        }
    }

    #[test]
    fn managed_build_start_never_overwrites_active_foreign_state() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, owner_spec, state_session_id, current_session_id, error_code) in [
            (
                "foreign-owner",
                Some(3327),
                "terminal-bridge-session",
                Some("terminal-bridge-session"),
                "build_state_owner_mismatch",
            ),
            (
                "foreign-session",
                Some(3403),
                "other-session",
                Some("terminal-bridge-session"),
                "build_state_session_mismatch",
            ),
            (
                "foreign-session-and-owner",
                Some(3327),
                "other-session",
                Some("terminal-bridge-session"),
                "build_state_session_mismatch",
            ),
        ] {
            let server = TerminalBridgeServer::start(
                StatusCode::OK,
                serde_json::json!({
                    "schema_version": 1,
                    "owner_number": 3403,
                    "work_id": "work-feature-build-start"
                }),
            );
            let (code, output, fixture) = run_active_action_with_identity(
                SkillStateAction::Start { spec: 3403 },
                Some(&server.forward_url),
                Some("terminal-secret"),
                true,
                owner_spec,
                state_session_id,
                current_session_id,
            );
            assert_eq!(code, 2, "{label}: {output}");
            let refusal: serde_json::Value = serde_json::from_str(output.trim())
                .unwrap_or_else(|error| panic!("{label}: typed refusal: {error}: {output}"));
            assert_eq!(refusal["error_code"], error_code, "{label}: {output}");
            let state = gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load preserved build state")
                .expect("preserved build state");
            assert!(state.active, "{label}: active state must remain active");
            assert_eq!(
                state.owner_spec, owner_spec,
                "{label}: owner must be preserved"
            );
            assert_eq!(
                state.session_id, state_session_id,
                "{label}: Session must be preserved"
            );
            server.assert_no_request();
        }
    }

    #[test]
    fn active_execution_record_keeps_missing_bridge_in_managed_preflight() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let (code, output, fixture) = run_start_action_with_execution_record(
            None,
            None,
            false,
            Some("start-bridge-session"),
            3403,
        );
        assert_eq!(code, 2, "{output}");
        let refusal: serde_json::Value =
            serde_json::from_str(output.trim()).expect("typed degraded-execution refusal");
        assert_eq!(refusal["error_code"], "relaunch_required");
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load rejected build state")
                .is_none(),
            "a degraded managed execution must not become standalone"
        );

        let (code, output, fixture) = run_start_action_with_execution_record(
            None,
            None,
            false,
            Some("start-bridge-session"),
            2359,
        );
        assert_eq!(code, 2, "{output}");
        let refusal: serde_json::Value =
            serde_json::from_str(output.trim()).expect("typed owner mismatch refusal");
        assert_eq!(refusal["error_code"], "execution_owner_mismatch");
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load mismatched build state")
                .is_none(),
            "the current Execution owner must constrain build.start"
        );

        let (code, output, fixture) = run_start_action_with_execution_record(
            None,
            None,
            false,
            Some("execution-owner-session"),
            3403,
        );
        assert_eq!(code, 2, "{output}");
        let refusal: serde_json::Value =
            serde_json::from_str(output.trim()).expect("typed Session mismatch refusal");
        assert_eq!(refusal["error_code"], "execution_session_mismatch");
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load foreign Session build state")
                .is_none(),
            "a foreign Session must not downgrade an existing Execution to standalone"
        );
    }

    #[test]
    fn managed_build_abort_recovers_only_proven_zero_mutation_orphan() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let missing_work = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing),
        );
        let (code, output, fixture) = run_active_action(
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("recover orphan".to_string()),
            },
            Some(&missing_work.forward_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 0, "{output}");
        assert!(output.contains("orphan_recovered"), "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load recovered build state")
                .expect("recovered build state")
                .active,
            "explicit Abort may clear a proven zero-mutation orphan"
        );
        missing_work.receive();

        let rejected = TerminalBridgeServer::start(
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "message": "assigned Work is not materialized"
            }),
        );
        let (code, output, fixture) = run_active_action(
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("recover rejected close".to_string()),
            },
            Some(&rejected.forward_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 0, "{output}");
        assert!(output.contains("orphan_recovered"), "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load rejected-close build state")
                .expect("rejected-close build state")
                .active
        );
        rejected.receive();

        let (code, output, fixture) = run_active_action(
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("recover before bridge send".to_string()),
            },
            Some("http://127.0.0.1:45123/internal/hook-live"),
            None,
            true,
        );
        assert_eq!(code, 0, "{output}");
        assert!(output.contains("orphan_recovered"), "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load not-sent build state")
                .expect("not-sent build state")
                .active,
            "incomplete bridge capability proves the terminal request was not sent"
        );
    }

    #[test]
    fn managed_build_abort_keeps_state_when_host_outcome_is_unknown() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let unavailable = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve unavailable bridge address");
        let port = unavailable
            .local_addr()
            .expect("unavailable bridge address")
            .port();
        drop(unavailable);
        let unavailable_url = format!("http://127.0.0.1:{port}/internal/hook-live");
        let (code, output, fixture) = run_active_action(
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("must stay active".to_string()),
            },
            Some(&unavailable_url),
            Some("terminal-secret"),
            true,
        );
        assert_eq!(code, 1, "{output}");
        assert_build_still_active(&fixture);

        for (label, status, body) in [
            (
                "wrong-terminal",
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::WrongTerminal),
            ),
            (
                "ambiguous-terminal",
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::AmbiguousTerminal),
            ),
            (
                "invalid-success",
                StatusCode::OK,
                serde_json::json!({
                    "schema_version": crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION + 1,
                    "owner_number": 3327,
                    "outcome": "assigned_work_missing"
                }),
            ),
            (
                "wrong-owner-receipt",
                StatusCode::OK,
                terminal_receipt_for_owner(
                    crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing,
                    2359,
                ),
            ),
            (
                "non-recoverable-rejection",
                StatusCode::CONFLICT,
                serde_json::json!({
                    "code": "provenance_mismatch",
                    "message": "untrusted Host diagnostic terminal-secret"
                }),
            ),
        ] {
            let server = TerminalBridgeServer::start(status, body);
            let (code, output, fixture) = run_active_action(
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("must stay active".to_string()),
                },
                Some(&server.forward_url),
                Some("terminal-secret"),
                true,
            );
            assert_eq!(code, 1, "{label}: {output}");
            assert!(
                !output.contains("terminal-secret"),
                "{label}: Host response must not reflect the bearer into diagnostics: {output}"
            );
            assert_build_still_active(&fixture);
            server.receive();
        }

        let missing_work = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing),
        );
        let (code, output, fixture) = run_active_action_with_owner(
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("must not recover unowned state".to_string()),
            },
            Some(&missing_work.forward_url),
            Some("terminal-secret"),
            true,
            None,
        );
        assert_eq!(code, 1, "{output}");
        assert_build_still_active(&fixture);
        missing_work.receive();

        for (label, current_session_id) in [
            ("foreign-session", Some("other-session")),
            ("missing-session", None),
        ] {
            let (code, output, fixture) = run_active_action_with_identity(
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("must preserve foreign state".to_string()),
                },
                None,
                None,
                true,
                Some(3327),
                "terminal-bridge-session",
                current_session_id,
            );
            assert_eq!(code, 1, "{label}: {output}");
            assert_build_still_active(&fixture);
        }
    }
}

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
    if matches!(&action, SkillStateAction::Complete { .. }) {
        let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
        if let Some(refusal) =
            crate::cli::verification_record::work_event_settlement_refusal(&worktree)
        {
            out.push_str(&format!("{VERB}: completion refused — {refusal}\n"));
            return Ok(2);
        }
    }
    if let Err(error) = record_current_work_terminal_before_finalize(env, &action) {
        out.push_str(&format!("{VERB}: Work lifecycle update failed: {error}\n"));
        return Ok(1);
    }
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
    let code = skill_state_runtime::run(env, action, SKILL_NAME, SKILL_DISPLAY, VERB, out)?;
    if code == 0 {
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

fn record_current_work_terminal_before_finalize<E: CliEnv>(
    env: &E,
    action: &SkillStateAction,
) -> Result<(), String> {
    let (spec, close_kind) = match action {
        SkillStateAction::Complete { spec } => (*spec, WorkTerminalKind::Done),
        SkillStateAction::Abort { spec, .. } => (*spec, WorkTerminalKind::Discarded),
        SkillStateAction::Start { .. } | SkillStateAction::Phase { .. } => return Ok(()),
    };
    let repo = env.repo_path();
    let state = gwt_core::skill_state::load(repo, SKILL_NAME).map_err(|error| error.to_string())?;
    let Some(state) = state else {
        return Ok(());
    };
    if state.owner_spec.is_some() && state.owner_spec != Some(spec) {
        return Ok(());
    }

    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .unwrap_or_default()
        .trim()
        .to_string();
    if session_id.is_empty() {
        return Ok(());
    }
    if !state.active || state.session_id.trim() != session_id {
        return Ok(());
    }

    if let Some(target) = crate::daemon_runtime::HookForwardTarget::from_env_strict()? {
        let observation = crate::observe_agent_runtime(repo).map_err(|error| error.to_string())?;
        let request = crate::AgentWorkTerminalizationRequest {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            claimed_session_id: session_id,
            observation,
            terminal_kind: match close_kind {
                WorkTerminalKind::Done => crate::AgentWorkTerminalKind::Done,
                WorkTerminalKind::Discarded => crate::AgentWorkTerminalKind::Discarded,
            },
        };
        let receipt =
            crate::daemon_runtime::send_work_terminalization_via_agent_bridge(&target, &request)?;
        return map_agent_terminal_outcome(receipt.outcome, close_kind);
    }
    if std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).is_some() {
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
        | gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::NoTarget => Ok(()),
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
) -> Result<(), String> {
    match outcome {
        crate::AgentWorkTerminalizationOutcome::Emitted
        | crate::AgentWorkTerminalizationOutcome::AlreadyMatching
        | crate::AgentWorkTerminalizationOutcome::NoTarget => Ok(()),
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

#[derive(Debug, Clone, Copy)]
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
        serde_json::to_value(crate::AgentWorkTerminalizationReceipt {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
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
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        gwt_core::skill_state::save(
            &fixture.repo,
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                active: true,
                owner_spec: Some(3327),
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: "terminal-bridge-session".to_string(),
            },
        )
        .expect("save active build state");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "terminal-bridge-session");
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
                    "schema_version": 2,
                    "outcome": "already_matching"
                }),
            ),
            (
                "wrong-terminal",
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::WrongTerminal),
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
}

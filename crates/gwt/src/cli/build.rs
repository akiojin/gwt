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
    let (spec, close_kind, abort_reason) = match action {
        SkillStateAction::Complete { spec } => (*spec, WorkTerminalKind::Done, None),
        SkillStateAction::Abort { spec, reason } => {
            (*spec, WorkTerminalKind::Discarded, reason.as_deref())
        }
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
        // SPEC-3431 (#3425 family): a session that was never bound by a launch
        // has no bound Work for the managed bridge to terminalize, so
        // demanding exact durable Host Work authority is unsatisfiable and
        // left the skill state permanently open (complete needs a receipt,
        // abort needed this authority — both require the missing binding).
        // Skipping only the managed-bridge terminalization is truthful: there
        // is no bound Work to mark. The legacy no-bridge path below still runs
        // for legacy Work state, and the Done path keeps its own receipt gate
        // in `build.complete`, which an unbound session still cannot pass.
        let session_toml = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
        if let Ok(session) = gwt_agent::Session::load(&session_toml) {
            if session.execution_binding.is_none() {
                return Ok(());
            }
        }
        let terminal_refusal = |refusal: String| {
            if matches!(close_kind, WorkTerminalKind::Discarded) {
                crate::cli::execution_state::terminal_recovery_refusal(repo, &session_id, &refusal)
            } else {
                refusal
            }
        };
        let compatibility_authority =
            crate::agent_project_state::snapshot_bound_terminal_compatibility_authority(
                repo,
                &session_id,
                match close_kind {
                    WorkTerminalKind::Done => crate::AgentWorkTerminalKind::Done,
                    WorkTerminalKind::Discarded => crate::AgentWorkTerminalKind::Discarded,
                },
            )
            .map_err(|error| terminal_refusal(error.to_string()))?
            .ok_or_else(|| {
                terminal_refusal(
                    "managed build terminalization requires an exact durable Host Work authority"
                        .to_string(),
                )
            })?;
        let observation = crate::observe_agent_runtime(repo).map_err(|error| error.to_string())?;
        let request = crate::AgentWorkTerminalizationRequest {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            claimed_session_id: session_id.clone(),
            observation,
            terminal_kind: match close_kind {
                WorkTerminalKind::Done => crate::AgentWorkTerminalKind::Done,
                WorkTerminalKind::Discarded => crate::AgentWorkTerminalKind::Discarded,
            },
        };
        let blocked_build_abort = compatibility_authority
            .requires_blocked_build_abort_bridge()
            .map_err(|error| terminal_refusal(error.to_string()))?;
        let receipt = if blocked_build_abort {
            let reason = abort_reason
                .filter(|reason| !reason.trim().is_empty())
                .ok_or_else(|| {
                    "Blocked build abort requires a non-empty reason before Host terminalization"
                        .to_string()
                })?;
            match crate::daemon_runtime::send_blocked_build_abort_terminalization_via_agent_bridge(
                &target,
                &crate::AgentBuildAbortTerminalizationRequest {
                    schema_version: crate::AGENT_BUILD_ABORT_TERMINALIZATION_SCHEMA_VERSION,
                    claimed_session_id: request.claimed_session_id.clone(),
                    owner_number: spec,
                    reason: reason.to_string(),
                    observation: request.observation.clone(),
                },
            ) {
                Ok(receipt) => receipt,
                Err(bridge_error) if bridge_error.is_missing_route_rejection() => {
                    return crate::agent_project_state::continue_bound_terminal_compatibility(
                        &compatibility_authority,
                        request,
                    )
                    .map(|_| ())
                    .map_err(|local_error| {
                        format!(
                            "{bridge_error}; local Blocked build abort reconciliation was also refused: {local_error}"
                        )
                    });
                }
                Err(bridge_error) => return Err(bridge_error.to_string()),
            }
        } else {
            crate::daemon_runtime::send_work_terminalization_via_agent_bridge(&target, &request)?
        };
        return match receipt.outcome {
            crate::AgentWorkTerminalizationOutcome::Emitted => {
                crate::agent_project_state::confirm_bound_terminal_compatibility_authority(
                    &compatibility_authority,
                    request,
                )
                .map_err(|error| {
                    format!(
                        "Host emitted a terminal event outside the canonical Work authority: {error}"
                    )
                })
            }
            crate::AgentWorkTerminalizationOutcome::AlreadyMatching
            | crate::AgentWorkTerminalizationOutcome::WrongTerminal
            | crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing
            | crate::AgentWorkTerminalizationOutcome::NoTarget => {
                crate::agent_project_state::continue_bound_terminal_compatibility(
                    &compatibility_authority,
                    request,
                )
                .map(|_| ())
            }
            crate::AgentWorkTerminalizationOutcome::AmbiguousTerminal => {
                map_agent_terminal_outcome(receipt.outcome, close_kind)
            }
        };
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
    use std::{
        io::{Read, Write},
        net::TcpListener as StdTcpListener,
        path::{Path, PathBuf},
        sync::{mpsc, Arc},
        thread::JoinHandle,
        time::Duration,
    };

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
        abort_rx: mpsc::Receiver<(HeaderMap, serde_json::Value)>,
        redirect_rx: mpsc::Receiver<HeaderMap>,
        forward_url: String,
    }

    struct RawTerminalBridgeServer {
        join_handle: Option<JoinHandle<()>>,
        request_rx: mpsc::Receiver<()>,
        forward_url: String,
    }

    impl RawTerminalBridgeServer {
        fn start(
            status: StatusCode,
            declared_content_length: Option<usize>,
            body: Vec<u8>,
        ) -> Self {
            let listener =
                StdTcpListener::bind(("127.0.0.1", 0)).expect("raw terminal bridge listener");
            let address = listener.local_addr().expect("raw terminal bridge address");
            let (request_tx, request_rx) = mpsc::channel();
            let join_handle = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("raw terminal bridge request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("raw terminal bridge read timeout");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut chunk).expect("read raw terminal request");
                    assert_ne!(read, 0, "terminal bridge request ended before its body");
                    request.extend_from_slice(&chunk[..read]);
                    let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = std::str::from_utf8(&request[..header_end])
                        .expect("raw terminal request headers");
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                request_tx.send(()).expect("capture raw terminal request");
                let reason = status.canonical_reason().unwrap_or("Rejected");
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
                    status.as_u16(),
                    reason
                )
                .expect("write raw terminal response status");
                if let Some(content_length) = declared_content_length {
                    write!(stream, "Content-Length: {content_length}\r\n")
                        .expect("write raw terminal response length");
                }
                write!(stream, "\r\n").expect("finish raw terminal response headers");
                // The client may reject an oversized Content-Length before
                // consuming the body and close the socket. That is the
                // behavior this fixture exercises, not a server failure.
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            });
            Self {
                join_handle: Some(join_handle),
                request_rx,
                forward_url: format!("http://127.0.0.1:{}/internal/hook-live", address.port()),
            }
        }

        fn receive(&self) {
            self.request_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("raw build abort bridge request");
        }
    }

    impl Drop for RawTerminalBridgeServer {
        fn drop(&mut self) {
            if let Some(join_handle) = self.join_handle.take() {
                join_handle.join().expect("join raw terminal bridge server");
            }
        }
    }

    #[derive(Clone)]
    struct TerminalBridgeState {
        tx: mpsc::Sender<(HeaderMap, serde_json::Value)>,
        abort_tx: mpsc::Sender<(HeaderMap, serde_json::Value)>,
        status: StatusCode,
        body: String,
        before_response: Arc<dyn Fn() + Send + Sync>,
        redirect_location: Option<String>,
        redirect_tx: mpsc::Sender<HeaderMap>,
    }

    impl TerminalBridgeServer {
        fn start(status: StatusCode, body: serde_json::Value) -> Self {
            Self::start_with_hook(status, body, || {})
        }

        fn start_with_hook(
            status: StatusCode,
            body: serde_json::Value,
            before_response: impl Fn() + Send + Sync + 'static,
        ) -> Self {
            Self::start_inner(status, body, before_response, None)
        }

        fn start_redirect() -> Self {
            Self::start_inner(
                StatusCode::TEMPORARY_REDIRECT,
                serde_json::Value::Null,
                || {},
                Some("/redirected-terminal".to_string()),
            )
        }

        fn start_inner(
            status: StatusCode,
            body: serde_json::Value,
            before_response: impl Fn() + Send + Sync + 'static,
            redirect_location: Option<String>,
        ) -> Self {
            let runtime = Runtime::new().expect("terminal bridge runtime");
            let listener = runtime
                .block_on(TcpListener::bind(("127.0.0.1", 0)))
                .expect("terminal bridge listener");
            let address = listener.local_addr().expect("terminal bridge address");
            let (tx, rx) = mpsc::channel();
            let (abort_tx, abort_rx) = mpsc::channel();
            let (redirect_tx, redirect_rx) = mpsc::channel();
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
                            (state.before_response)();
                            let mut response = (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response();
                            if let Some(location) = state.redirect_location.as_deref() {
                                response.headers_mut().insert(
                                    axum::http::header::LOCATION,
                                    axum::http::HeaderValue::from_str(location)
                                        .expect("valid redirect location"),
                                );
                            }
                            response
                        },
                    ),
                )
                .route(
                    "/internal/build-abort-terminalization",
                    post(
                        |headers: HeaderMap,
                         State(state): State<TerminalBridgeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .abort_tx
                                .send((headers, body))
                                .expect("capture build abort bridge request");
                            (state.before_response)();
                            let mut response = (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response();
                            if let Some(location) = state.redirect_location.as_deref() {
                                response.headers_mut().insert(
                                    axum::http::header::LOCATION,
                                    axum::http::HeaderValue::from_str(location)
                                        .expect("valid redirect location"),
                                );
                            }
                            response
                        },
                    ),
                )
                .route(
                    "/redirected-terminal",
                    post(
                        |headers: HeaderMap, State(state): State<TerminalBridgeState>| async move {
                            state
                                .redirect_tx
                                .send(headers)
                                .expect("capture redirected terminal request");
                            StatusCode::OK
                        },
                    ),
                )
                .with_state(TerminalBridgeState {
                    tx,
                    abort_tx,
                    status,
                    body: body.to_string(),
                    before_response: Arc::new(before_response),
                    redirect_location,
                    redirect_tx,
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
                abort_rx,
                redirect_rx,
                forward_url: format!("http://127.0.0.1:{}/internal/hook-live", address.port()),
            }
        }

        fn receive(&self) -> (HeaderMap, serde_json::Value) {
            self.rx
                .recv_timeout(Duration::from_secs(2))
                .expect("terminal bridge request")
        }

        fn receive_abort(&self) -> (HeaderMap, serde_json::Value) {
            self.abort_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("build abort bridge request")
        }

        fn assert_no_request(&self) {
            assert!(
                matches!(self.rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "terminal bridge must not receive a request"
            );
        }

        fn assert_no_abort_request(&self) {
            assert!(
                matches!(self.abort_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "build abort bridge must not receive a request"
            );
        }

        fn assert_no_redirect(&self) {
            assert!(
                matches!(self.redirect_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "terminal bridge client must not follow redirects"
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

    struct BoundTerminalFixture {
        git: crate::cli::verification_record::tests::WorkEventGitFixture,
        session: gwt_agent::Session,
        binding: gwt_agent::SessionExecutionBinding,
        work_id: String,
    }

    impl BoundTerminalFixture {
        fn new() -> Self {
            const OWNER_NUMBER: u64 = 3327;
            let git = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
            let branch =
                gwt_core::process::run_git_logged(&["branch", "--show-current"], Some(&git.repo))
                    .expect("read fixture branch");
            assert!(branch.status.success(), "fixture branch must be readable");
            let branch = String::from_utf8(branch.stdout)
                .expect("fixture branch is UTF-8")
                .trim()
                .to_string();
            let mut session =
                gwt_agent::Session::new(&git.repo, &branch, gwt_agent::AgentId::Codex);
            session.id = "terminal-bridge-session".to_string();
            session.project_state_root = Some(git.repo.clone());
            session.linked_issue_number = Some(OWNER_NUMBER);
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save unbound Session fixture");

            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: OWNER_NUMBER,
            };
            crate::cli::execution_state::materialize_at_launch(
                &git.repo,
                owner.kind,
                owner.number,
                &session.id,
                "gwt-execute",
                false,
            )
            .expect("materialize execution control fixture");
            crate::cli::execution_state::ensure_generation_ledger(
                &git.repo,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Live,
            )
            .expect("materialize generation ledger fixture");
            let identity = crate::cli::execution_state::current_execution_binding(&git.repo, owner)
                .expect("read fixture execution binding")
                .expect("fixture execution binding");
            let binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session.id.clone(),
                repo_hash: session
                    .repo_hash
                    .clone()
                    .expect("fixture repository identity"),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            };
            session
                .set_execution_binding(Some(binding.clone()))
                .expect("bind fixture Session");
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save bound Session fixture");

            let work_id = "work-terminal-bridge-canonical".to_string();
            let now = chrono::Utc::now();
            let mut current =
                gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&git.repo);
            current
                .agents
                .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                    session_id: session.id.clone(),
                    window_id: Some("project::terminal-bridge".to_string()),
                    agent_id: session.agent_id.command().to_string(),
                    display_name: session.agent_id.display_name().to_string(),
                    status_category:
                        gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                    current_focus: None,
                    title_summary: None,
                    worktree_path: Some(git.repo.clone()),
                    branch: Some(branch),
                    last_board_entry_id: None,
                    last_board_entry_kind: None,
                    coordination_scope: None,
                    affiliation_status:
                        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                    workspace_id: Some(work_id.clone()),
                    updated_at: now,
                });
            gwt_core::workspace_projection::save_workspace_projection(&git.repo, &current)
                .expect("save canonical current fixture");

            let mut work_items = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
            let mut start = gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Start,
                &work_id,
                now,
            );
            start.title = Some("Canonical terminal bridge Work".to_string());
            start.owner = Some(format!("Issue #{}", owner.number));
            start.status_category =
                Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
            start.agent_session_id = Some(session.id.clone());
            start.agent_id = Some(session.agent_id.command().to_string());
            start.execution_container = Some(
                gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                    branch: Some(session.branch.clone()),
                    worktree_path: Some(git.repo.clone()),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                },
            );
            work_items.apply_event(start);
            let works_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&git.repo);
            gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                &works_path,
                &work_items,
            )
            .expect("save canonical WorkItems fixture");

            gwt_core::skill_state::save(
                &git.repo,
                SKILL_NAME,
                &gwt_core::skill_state::SkillState {
                    active: true,
                    owner_spec: Some(OWNER_NUMBER),
                    started_at: now,
                    phase: Some("pr".to_string()),
                    session_id: session.id.clone(),
                },
            )
            .expect("save active build fixture");

            Self {
                git,
                session,
                binding,
                work_id,
            }
        }

        fn terminalize_canonical_done(&self) {
            let receipt = crate::apply_bound_authenticated_work_terminalization(
                &self.git.repo,
                &self.session.id,
                &self.binding,
                crate::AgentWorkTerminalizationRequest {
                    schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                    claimed_session_id: self.session.id.clone(),
                    observation: crate::observe_agent_runtime(&self.git.repo)
                        .expect("observe fixture runtime"),
                    terminal_kind: crate::AgentWorkTerminalKind::Done,
                },
            )
            .expect("terminalize canonical fixture");
            assert_eq!(
                receipt.outcome,
                crate::AgentWorkTerminalizationOutcome::Emitted
            );
        }

        fn settle_execution_blocked(&self) {
            assert!(matches!(
                crate::cli::execution_state::settle(
                    &self.git.repo,
                    &self.session.id,
                    crate::cli::execution_state::ExecutionSettlement::Blocked {
                        reason: "canonical verification is externally blocked".to_string(),
                        missing_verification: Some("full matrix".to_string()),
                    },
                )
                .expect("settle fixture execution"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ));
        }

        fn canonical_work(&self) -> gwt_core::workspace_projection::WorkspaceWorkItem {
            gwt_core::workspace_projection::load_workspace_work_items(&self.git.repo)
                .expect("load canonical WorkItems")
                .expect("canonical WorkItems")
                .work_items
                .into_iter()
                .find(|work| work.id == self.work_id)
                .expect("canonical Work")
        }
    }

    struct SplitRootTerminalFixture {
        _temp: tempfile::TempDir,
        project_state_root: PathBuf,
        worktree: PathBuf,
        session: gwt_agent::Session,
        binding: gwt_agent::SessionExecutionBinding,
        work_id: String,
        legacy_work_items_path: PathBuf,
        legacy_close_path: PathBuf,
    }

    impl SplitRootTerminalFixture {
        fn new() -> Self {
            const OWNER_NUMBER: u64 = 3327;
            const BRANCH: &str = "work/20260601-0934";
            const REMOTE: &str = "https://example.invalid/acme/terminal-split-root.git";
            let temp = tempfile::tempdir().expect("split-root terminal fixture");
            let project_state_root = temp.path().join("workspace-home");
            let bootstrap = project_state_root.join("bootstrap");
            let worktree = project_state_root.join("work").join("issue-3327");
            std::fs::create_dir_all(&bootstrap).expect("split-root bootstrap");
            crate::cli::trusted_store::init_git_repo_with_origin(&bootstrap);
            run_terminal_fixture_git(&["checkout", "-b", BRANCH], &bootstrap);
            let bare = project_state_root.join("gwt.git");
            run_terminal_fixture_git(
                &[
                    "clone",
                    "--bare",
                    bootstrap.to_str().expect("bootstrap path"),
                    bare.to_str().expect("bare path"),
                ],
                &project_state_root,
            );
            run_terminal_fixture_git(&["remote", "set-url", "origin", REMOTE], &bare);
            std::fs::create_dir_all(worktree.parent().expect("worktree parent"))
                .expect("worktree parent");
            run_terminal_fixture_git(
                &[
                    "worktree",
                    "add",
                    worktree.to_str().expect("worktree path"),
                    BRANCH,
                ],
                &bare,
            );
            let project_state_root =
                dunce::canonicalize(&project_state_root).expect("canonical Project State root");
            let worktree = dunce::canonicalize(&worktree).expect("canonical linked worktree");

            let session_id = "terminal-split-root-session";
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: OWNER_NUMBER,
            };
            crate::cli::execution_state::materialize_at_launch(
                &worktree,
                owner.kind,
                owner.number,
                session_id,
                "$gwt-execute #3327",
                false,
            )
            .expect("materialize split-root execution");
            crate::cli::execution_state::ensure_generation_ledger(
                &worktree,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Live,
            )
            .expect("materialize split-root generation ledger");
            let identity = crate::cli::execution_state::current_execution_binding(&worktree, owner)
                .expect("read split-root execution binding")
                .expect("split-root execution binding");
            let mut session = gwt_agent::Session::new(&worktree, BRANCH, gwt_agent::AgentId::Codex);
            session.id = session_id.to_string();
            session.project_state_root = Some(project_state_root.clone());
            session.linked_issue_number = Some(OWNER_NUMBER);
            let binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session.id.clone(),
                repo_hash: session.repo_hash.clone().expect("split-root repo identity"),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            };
            session
                .set_execution_binding(Some(binding.clone()))
                .expect("bind split-root Session");
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save split-root Session");

            let work_id = "work-terminal-split-root-canonical".to_string();
            let now = chrono::Utc::now();
            let mut current =
                gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                    &project_state_root,
                );
            current
                .agents
                .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                    session_id: session.id.clone(),
                    window_id: Some("project::terminal-split-root".to_string()),
                    agent_id: session.agent_id.command().to_string(),
                    display_name: session.agent_id.display_name().to_string(),
                    status_category:
                        gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                    current_focus: None,
                    title_summary: None,
                    worktree_path: Some(worktree.clone()),
                    branch: Some(BRANCH.to_string()),
                    last_board_entry_id: None,
                    last_board_entry_kind: None,
                    coordination_scope: None,
                    affiliation_status:
                        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                    workspace_id: Some(work_id.clone()),
                    updated_at: now,
                });
            gwt_core::workspace_projection::save_workspace_projection(
                &project_state_root,
                &current,
            )
            .expect("save split-root canonical current");

            let mut start = gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Start,
                &work_id,
                now,
            );
            start.title = Some("Split-root canonical terminal Work".to_string());
            start.owner = Some(format!("Issue #{}", owner.number));
            start.status_category =
                Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
            start.agent_session_id = Some(session.id.clone());
            start.agent_id = Some(session.agent_id.command().to_string());
            start.execution_container = Some(
                gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                    branch: Some(BRANCH.to_string()),
                    worktree_path: Some(worktree.clone()),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                },
            );
            let mut canonical = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
            canonical.apply_event(start);
            let canonical_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_state_root);
            gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                &canonical_path,
                &canonical,
            )
            .expect("save split-root canonical WorkItems");

            let mut legacy = canonical.clone();
            let mut discard = gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Discard,
                &work_id,
                now + chrono::Duration::seconds(1),
            );
            discard.agent_session_id = Some(session.id.clone());
            legacy.apply_event(discard.clone());
            let legacy_work_items_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&worktree);
            assert_ne!(canonical_path, legacy_work_items_path);
            gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                &legacy_work_items_path,
                &legacy,
            )
            .expect("save stale linked-worktree WorkItems");
            let legacy_close_path =
                gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&worktree);
            std::fs::create_dir_all(legacy_close_path.parent().expect("legacy close parent"))
                .expect("legacy close parent");
            std::fs::write(
                &legacy_close_path,
                format!(
                    "{}\n",
                    serde_json::to_string(&discard).expect("serialize legacy discard")
                ),
            )
            .expect("save stale linked-worktree close ledger");

            gwt_core::skill_state::save(
                &worktree,
                SKILL_NAME,
                &gwt_core::skill_state::SkillState {
                    active: true,
                    owner_spec: Some(OWNER_NUMBER),
                    started_at: now,
                    phase: Some("pr".to_string()),
                    session_id: session.id.clone(),
                },
            )
            .expect("save split-root build state");

            Self {
                _temp: temp,
                project_state_root,
                worktree,
                session,
                binding,
                work_id,
                legacy_work_items_path,
                legacy_close_path,
            }
        }

        fn terminalize_canonical_done(&self) {
            let receipt = crate::apply_bound_authenticated_work_terminalization(
                &self.project_state_root,
                &self.session.id,
                &self.binding,
                crate::AgentWorkTerminalizationRequest {
                    schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                    claimed_session_id: self.session.id.clone(),
                    observation: crate::observe_agent_runtime(&self.worktree)
                        .expect("observe split-root fixture runtime"),
                    terminal_kind: crate::AgentWorkTerminalKind::Done,
                },
            )
            .expect("terminalize split-root canonical Work");
            assert_eq!(
                receipt.outcome,
                crate::AgentWorkTerminalizationOutcome::Emitted
            );
        }
    }

    fn run_terminal_fixture_git(args: &[&str], cwd: &Path) {
        let output = gwt_core::process::run_git_logged(args, Some(cwd)).expect("run fixture git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn with_terminal_bridge_env<T>(
        fixture: &BoundTerminalFixture,
        server: &TerminalBridgeServer,
        operation: impl FnOnce(&crate::cli::TestEnv) -> T,
    ) -> T {
        with_terminal_bridge_env_for(&fixture.git.repo, &fixture.session.id, server, operation)
    }

    fn with_terminal_bridge_env_for<T>(
        repo: &Path,
        session_id: &str,
        server: &TerminalBridgeServer,
        operation: impl FnOnce(&crate::cli::TestEnv) -> T,
    ) -> T {
        with_terminal_bridge_url_env_for(repo, session_id, &server.forward_url, operation)
    }

    fn with_terminal_bridge_url_env_for<T>(
        repo: &Path,
        session_id: &str,
        forward_url: &str,
        operation: impl FnOnce(&crate::cli::TestEnv) -> T,
    ) -> T {
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let _forward_url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, forward_url);
        let _forward_token =
            ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "terminal-secret");
        let _runtime = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            repo.join("managed-runtime.json"),
        );
        operation(&crate::cli::TestEnv::new(repo.to_path_buf()))
    }

    #[test]
    fn managed_build_complete_confirms_canonical_done_after_old_host_wrong_terminal() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.terminalize_canonical_done();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let before = std::fs::read(&works_path).expect("snapshot canonical Done WorkItems");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::WrongTerminal),
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete { spec: 3327 },
            )
        });

        assert!(result.is_ok(), "canonical Done must win: {result:?}");
        assert_eq!(
            std::fs::read(&works_path).expect("read canonical WorkItems after retry"),
            before,
            "confirm-only compatibility must not append another terminal event"
        );
        assert!(fixture.canonical_work().is_terminal());
        assert!(!fixture.canonical_work().discarded);
        let (_, request) = server.receive();
        assert_eq!(request["terminal_kind"], "done");
    }

    #[test]
    fn managed_build_complete_preserves_opposite_linked_worktree_legacy_terminal() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = SplitRootTerminalFixture::new();
        fixture.terminalize_canonical_done();
        let canonical_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(
            &fixture.project_state_root,
        );
        let canonical_before =
            std::fs::read(&canonical_path).expect("snapshot repo-global canonical WorkItems");
        let legacy_before = [
            std::fs::read(&fixture.legacy_work_items_path)
                .expect("snapshot linked-worktree legacy WorkItems"),
            std::fs::read(&fixture.legacy_close_path)
                .expect("snapshot linked-worktree legacy close ledger"),
        ];
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::WrongTerminal),
        );

        let result =
            with_terminal_bridge_env_for(&fixture.worktree, &fixture.session.id, &server, |env| {
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Complete { spec: 3327 },
                )
            });

        assert!(
            result.is_ok(),
            "split-root canonical Done must win: {result:?}"
        );
        assert_eq!(
            std::fs::read(&canonical_path).expect("read repo-global canonical WorkItems"),
            canonical_before,
            "confirm-only must not duplicate the canonical terminal"
        );
        assert_eq!(
            [
                std::fs::read(&fixture.legacy_work_items_path)
                    .expect("read linked-worktree legacy WorkItems"),
                std::fs::read(&fixture.legacy_close_path)
                    .expect("read linked-worktree legacy close ledger"),
            ],
            legacy_before,
            "new-client reconciliation must preserve old-Host legacy terminal bytes"
        );
        let canonical =
            gwt_core::workspace_projection::load_workspace_work_items(&fixture.project_state_root)
                .expect("load canonical split-root WorkItems")
                .expect("canonical split-root WorkItems");
        let work = canonical
            .work_items
            .iter()
            .find(|work| work.id == fixture.work_id)
            .expect("canonical split-root Work");
        assert!(work.is_terminal());
        assert!(!work.discarded);
        server.receive();
    }

    /// SPEC-3431 (#3425 family): a session without an execution binding can
    /// still abort its own build state.
    ///
    /// A binding exists only when gwt's launch materialization wrote it; a
    /// resumed successor session has none. Such a session also has no bound
    /// Work to terminalize — yet the abort path demanded exact durable Host
    /// Work authority and failed with "durable Session ... has no execution
    /// binding". Combined with `build.complete` requiring a Work event
    /// receipt (which likewise needs the binding), a skill state opened by an
    /// unbound session could be neither settled nor abandoned, so the Stop
    /// hook blocked that session forever. Observed live on work/issue-3431.
    ///
    /// Fail-open is safe here: skipping means "no bound Work exists, so there
    /// is nothing to mark", and the Done path keeps its own receipt gate in
    /// `build.complete`, which an unbound session still cannot pass.
    #[test]
    fn unbound_session_abort_skips_the_terminal_work_update() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let git = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let branch =
            gwt_core::process::run_git_logged(&["branch", "--show-current"], Some(&git.repo))
                .expect("read fixture branch");
        let branch = String::from_utf8(branch.stdout)
            .expect("utf8")
            .trim()
            .to_string();
        let mut session = gwt_agent::Session::new(&git.repo, &branch, gwt_agent::AgentId::Codex);
        session.id = "unbound-successor-session".to_string();
        session.project_state_root = Some(git.repo.clone());
        session.linked_issue_number = Some(3431);
        assert!(
            session.execution_binding.is_none(),
            "precondition: this session was never bound by a launch"
        );
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save unbound session");
        gwt_core::skill_state::save(
            &git.repo,
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                active: true,
                owner_spec: Some(3431),
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: session.id.clone(),
            },
        )
        .expect("open the build state as this session");

        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let result = with_terminal_bridge_env_for(&git.repo, &session.id, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Abort {
                    spec: 3431,
                    reason: Some("opened only to probe verify authority".to_string()),
                },
            )
        });

        assert!(
            result.is_ok(),
            "an unbound session must be able to abandon its own build state: {result:?}"
        );
    }

    #[test]
    fn managed_build_abort_closes_canonical_active_after_old_host_no_write_outcome() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        assert!(!fixture.canonical_work().is_terminal());
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("cancelled".to_string()),
                },
            )
        });

        assert!(result.is_ok(), "canonical abort must converge: {result:?}");
        let work = fixture.canonical_work();
        assert!(work.is_terminal());
        assert!(
            work.discarded,
            "canonical Work must be discarded exactly once"
        );
        let (_, request) = server.receive();
        assert_eq!(request["terminal_kind"], "discarded");
    }

    #[test]
    fn blocked_execution_allows_build_abort_but_rejects_build_complete() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let abort_fixture = BoundTerminalFixture::new();
        let _session = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            abort_fixture.session.id.clone(),
        );
        let mut start_env = crate::cli::TestEnv::new(abort_fixture.git.repo.clone());
        let mut start_output = String::new();
        assert_eq!(
            run(
                &mut start_env,
                SkillStateAction::Start { spec: 3327 },
                &mut start_output,
            )
            .expect("run build.start"),
            0,
            "{start_output}"
        );
        assert!(matches!(
            crate::cli::execution_state::settle(
                &abort_fixture.git.repo,
                &abort_fixture.session.id,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "canonical verification is externally blocked".to_string(),
                    missing_verification: Some("full matrix".to_string()),
                },
            )
            .expect("settle abort fixture execution"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let abort_server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let abort_repo = abort_fixture.git.repo.clone();
        let (abort_code, abort_output) =
            with_terminal_bridge_env(&abort_fixture, &abort_server, |_| {
                let mut env = crate::cli::TestEnv::new(abort_repo);
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("blocked verification cannot proceed".to_string()),
                    },
                    &mut output,
                )
                .expect("run build.abort after execution.blocked");
                (code, output)
            });

        assert_eq!(abort_code, 0, "{abort_output}");
        assert!(
            !gwt_core::skill_state::load(&abort_fixture.git.repo, SKILL_NAME)
                .expect("load aborted build state")
                .expect("aborted build state")
                .active,
            "build.abort must close the stranded lifecycle"
        );
        let aborted_work = abort_fixture.canonical_work();
        assert!(aborted_work.is_terminal());
        assert!(aborted_work.discarded);
        let (_, abort_request) = abort_server.receive_abort();
        assert_eq!(abort_request["owner_number"], 3327);
        assert_eq!(
            abort_request["reason"],
            "blocked verification cannot proceed"
        );
        assert!(abort_request.get("terminal_kind").is_none());

        let complete_fixture = BoundTerminalFixture::new();
        let mut start_env = crate::cli::TestEnv::new(complete_fixture.git.repo.clone());
        let mut start_output = String::new();
        assert_eq!(
            run(
                &mut start_env,
                SkillStateAction::Start { spec: 3327 },
                &mut start_output,
            )
            .expect("run build.start"),
            0,
            "{start_output}"
        );
        assert!(matches!(
            crate::cli::execution_state::settle(
                &complete_fixture.git.repo,
                &complete_fixture.session.id,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "canonical verification is externally blocked".to_string(),
                    missing_verification: Some("full matrix".to_string()),
                },
            )
            .expect("settle complete fixture execution"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let complete_server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let complete_repo = complete_fixture.git.repo.clone();
        let (complete_code, complete_output) =
            with_terminal_bridge_env(&complete_fixture, &complete_server, |_| {
                let mut env = crate::cli::TestEnv::new(complete_repo);
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Complete { spec: 3327 },
                    &mut output,
                )
                .expect("run build.complete after execution.blocked");
                (code, output)
            });

        assert_ne!(complete_code, 0, "{complete_output}");
        assert_build_still_active(&complete_fixture.git);
        assert!(!complete_fixture.canonical_work().is_terminal());
        complete_server.assert_no_request();
    }

    #[test]
    fn blocked_build_abort_transport_failure_never_falls_back_locally() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();

        let unavailable = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve unavailable bridge address");
        let port = unavailable
            .local_addr()
            .expect("unavailable bridge address")
            .port();
        drop(unavailable);
        let unavailable_url = format!("http://127.0.0.1:{port}/internal/hook-live");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &fixture.session.id);
        let _forward_url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &unavailable_url);
        let _forward_token =
            ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "terminal-secret");
        let _runtime = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            fixture.git.repo.join("managed-runtime.json"),
        );
        let mut env = crate::cli::TestEnv::new(fixture.git.repo.clone());
        let mut output = String::new();

        let code = run(
            &mut env,
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("blocked verification cannot proceed".to_string()),
            },
            &mut output,
        )
        .expect("run build.abort with unavailable Host bridge");

        assert_ne!(code, 0, "{output}");
        assert!(
            output.contains("outcome may be unknown"),
            "transport failure must remain fail-closed: {output}"
        );
        assert_build_still_active(&fixture.git);
        assert!(
            !fixture.canonical_work().is_terminal(),
            "unknown Host outcome must not trigger a local terminal write"
        );
    }

    #[test]
    fn blocked_build_abort_rejects_blank_reason_before_host_or_local_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();
        let server = TerminalBridgeServer::start(StatusCode::NOT_FOUND, serde_json::Value::Null);
        let repo = fixture.git.repo.clone();

        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("   ".to_string()),
                },
                &mut output,
            )
            .expect("run build.abort with blank reason");
            (code, output)
        });

        assert_ne!(code, 0, "{output}");
        assert!(output.contains("non-empty reason"), "{output}");
        assert_build_still_active(&fixture.git);
        assert!(!fixture.canonical_work().is_terminal());
        server.assert_no_abort_request();
    }

    #[test]
    fn blocked_build_abort_unreadable_or_oversized_rejection_body_fails_closed() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let limit = 64 * 1024;

        for (label, declared_content_length, body) in [
            ("truncated", Some(64), b"{}".to_vec()),
            ("declared-oversized", Some(limit + 1), vec![b' '; limit + 1]),
            ("streamed-oversized", None, vec![b' '; limit + 1]),
        ] {
            let fixture = BoundTerminalFixture::new();
            fixture.settle_execution_blocked();
            let server = RawTerminalBridgeServer::start(
                StatusCode::NOT_FOUND,
                declared_content_length,
                body,
            );
            let repo = fixture.git.repo.clone();

            let (code, output) = with_terminal_bridge_url_env_for(
                &fixture.git.repo,
                &fixture.session.id,
                &server.forward_url,
                |_| {
                    let mut env = crate::cli::TestEnv::new(repo);
                    let mut output = String::new();
                    let code = run(
                        &mut env,
                        SkillStateAction::Abort {
                            spec: 3327,
                            reason: Some("blocked verification cannot proceed".to_string()),
                        },
                        &mut output,
                    )
                    .expect("run build.abort after unsafe Host rejection body");
                    (code, output)
                },
            );

            assert_ne!(code, 0, "{label}: {output}");
            assert!(output.contains("transport_failure"), "{label}: {output}");
            assert_build_still_active(&fixture.git);
            assert!(
                !fixture.canonical_work().is_terminal(),
                "{label}: unsafe rejection body must not trigger a local terminal write"
            );
            server.receive();
        }
    }

    #[test]
    fn blocked_build_abort_missing_host_route_falls_back_locally() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        for status in [StatusCode::NOT_FOUND, StatusCode::METHOD_NOT_ALLOWED] {
            let fixture = BoundTerminalFixture::new();
            fixture.settle_execution_blocked();
            let server = TerminalBridgeServer::start(status, serde_json::Value::Null);
            let repo = fixture.git.repo.clone();

            let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
                let mut env = crate::cli::TestEnv::new(repo);
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("blocked verification cannot proceed".to_string()),
                    },
                    &mut output,
                )
                .expect("run build.abort after explicit Host rejection");
                (code, output)
            });

            assert_eq!(code, 0, "{status}: {output}");
            assert!(
                !gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                    .expect("load reconciled build state")
                    .expect("reconciled build state")
                    .active
            );
            let work = fixture.canonical_work();
            assert!(work.is_terminal());
            assert!(work.discarded);
            server.receive_abort();
        }
    }

    #[test]
    fn blocked_build_abort_host_denials_and_unknown_responses_fail_closed() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, status, body) in [
            (
                "unauthorized",
                StatusCode::UNAUTHORIZED,
                serde_json::json!({
                    "code": "invalid_request",
                    "reason": "invalid_request",
                    "message": "agent capability is invalid"
                }),
            ),
            (
                "forbidden",
                StatusCode::FORBIDDEN,
                serde_json::json!({
                    "code": "invalid_request",
                    "reason": "invalid_request",
                    "message": "agent capability is forbidden"
                }),
            ),
            (
                "authority-mismatch",
                StatusCode::CONFLICT,
                serde_json::json!({
                    "code": "execution_binding_mismatch",
                    "reason": "authority_mismatch",
                    "message": "execution binding is stale"
                }),
            ),
            (
                "unknown-conflict",
                StatusCode::CONFLICT,
                serde_json::json!({
                    "code": "future_conflict",
                    "reason": "future_conflict",
                    "message": "future Host rejection"
                }),
            ),
            (
                "internal",
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({
                    "code": "internal",
                    "reason": "internal",
                    "message": "Host mutation outcome is unknown"
                }),
            ),
            (
                "unsupported-receipt",
                StatusCode::OK,
                serde_json::json!({
                    "schema_version": 2,
                    "outcome": "already_matching"
                }),
            ),
            ("invalid-receipt", StatusCode::OK, serde_json::Value::Null),
        ] {
            let fixture = BoundTerminalFixture::new();
            fixture.settle_execution_blocked();
            let server = TerminalBridgeServer::start(status, body);
            let repo = fixture.git.repo.clone();

            let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
                let mut env = crate::cli::TestEnv::new(repo);
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("blocked verification cannot proceed".to_string()),
                    },
                    &mut output,
                )
                .expect("run build.abort after Host denial");
                (code, output)
            });

            assert_ne!(code, 0, "{label}: {output}");
            assert_build_still_active(&fixture.git);
            assert!(
                !fixture.canonical_work().is_terminal(),
                "{label}: Host denial or unknown outcome must not trigger a local terminal write"
            );
            server.receive_abort();
        }
    }

    #[test]
    fn blocked_build_abort_redirect_never_falls_back_or_forwards_its_bearer() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();
        let server = TerminalBridgeServer::start_redirect();
        let repo = fixture.git.repo.clone();

        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("blocked verification cannot proceed".to_string()),
                },
                &mut output,
            )
            .expect("run redirected build.abort");
            (code, output)
        });

        assert_ne!(code, 0, "{output}");
        assert_build_still_active(&fixture.git);
        assert!(!fixture.canonical_work().is_terminal());
        server.receive_abort();
        server.assert_no_redirect();
    }

    #[test]
    fn managed_build_complete_converges_after_each_schema_valid_host_no_write_outcome() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for outcome in [
            crate::AgentWorkTerminalizationOutcome::AlreadyMatching,
            crate::AgentWorkTerminalizationOutcome::WrongTerminal,
            crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing,
            crate::AgentWorkTerminalizationOutcome::NoTarget,
        ] {
            let fixture = BoundTerminalFixture::new();
            let initial_event_count = fixture.canonical_work().events.len();
            let server = TerminalBridgeServer::start(StatusCode::OK, terminal_receipt(outcome));

            let result = with_terminal_bridge_env(&fixture, &server, |env| {
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Complete { spec: 3327 },
                )
            });

            assert!(result.is_ok(), "{outcome:?} must converge: {result:?}");
            let work = fixture.canonical_work();
            assert!(work.is_terminal());
            assert!(!work.discarded);
            assert_eq!(
                work.events.len(),
                initial_event_count + 1,
                "{outcome:?} must append exactly one canonical terminal event"
            );
            server.receive();
        }
    }

    #[test]
    fn managed_build_rejects_host_emitted_without_exact_canonical_readback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let close_path =
            gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&fixture.git.repo);
        let mut unprojected = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            &fixture.work_id,
            chrono::Utc::now(),
        );
        unprojected.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
        unprojected.agent_session_id = Some(fixture.session.id.clone());
        std::fs::create_dir_all(close_path.parent().expect("canonical close parent"))
            .expect("canonical close parent");
        std::fs::write(
            &close_path,
            format!(
                "{}\n",
                serde_json::to_string(&unprojected).expect("serialize unprojected close")
            ),
        )
        .expect("seed unprojected canonical close");
        let before = std::fs::read(&works_path).expect("snapshot active canonical WorkItems");
        let close_before =
            std::fs::read(&close_path).expect("snapshot unprojected canonical close");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
        );

        let error = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete { spec: 3327 },
            )
        })
        .expect_err("Host Emitted without canonical readback must fail closed");

        assert!(
            error.contains("outside the canonical Work authority"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(&works_path).expect("read active canonical WorkItems"),
            before,
            "post-Emitted confirmation must never replay a local terminal event"
        );
        assert_eq!(
            std::fs::read(&close_path).expect("read unprojected canonical close"),
            close_before,
            "post-Emitted confirmation must be a pure readback"
        );
        assert_build_still_active(&fixture.git);
        server.receive();
    }

    #[test]
    fn managed_build_rejects_opposite_and_ambiguous_canonical_terminal_before_bridge() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for ambiguous in [false, true] {
            let fixture = BoundTerminalFixture::new();
            fixture.terminalize_canonical_done();
            if ambiguous {
                let works_path =
                    gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
                let mut works =
                    gwt_core::workspace_projection::load_workspace_work_items(&fixture.git.repo)
                        .expect("load canonical WorkItems")
                        .expect("canonical WorkItems");
                works
                    .work_items
                    .iter_mut()
                    .find(|work| work.id == fixture.work_id)
                    .expect("canonical Work")
                    .discarded = true;
                gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                    &works_path,
                    &works,
                )
                .expect("seed ambiguous canonical terminal");
            }
            let works_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
            let before = std::fs::read(&works_path).expect("snapshot canonical terminal");
            let server = TerminalBridgeServer::start(
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
            );

            let error = with_terminal_bridge_env(&fixture, &server, |env| {
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("cancelled".to_string()),
                    },
                )
            })
            .expect_err("opposite or ambiguous canonical terminal must fail before bridge");

            assert!(
                error.contains(if ambiguous { "ambiguous" } else { "opposite" }),
                "{error}"
            );
            assert_eq!(
                std::fs::read(&works_path).expect("read canonical terminal"),
                before
            );
            assert_build_still_active(&fixture.git);
            server.assert_no_request();
        }
    }

    #[test]
    fn managed_build_rejects_binding_rotation_between_bridge_and_exact_commit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let before = std::fs::read(&works_path).expect("snapshot active canonical WorkItems");
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session_id = fixture.session.id.clone();
        let server = TerminalBridgeServer::start_with_hook(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
            move || {
                gwt_agent::rotate_session_execution_capability(&sessions_dir, &session_id)
                    .expect("rotate capability before terminal continuation");
            },
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete { spec: 3327 },
            )
        });

        assert!(result.is_err(), "rotated binding must fail closed");
        assert_eq!(
            std::fs::read(&works_path).expect("read active canonical WorkItems"),
            before,
            "binding rotation must preserve canonical Work bytes"
        );
        assert_build_still_active(&fixture.git);
        server.receive();
    }

    #[test]
    fn managed_build_accepts_host_emitted_only_after_exact_canonical_readback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let initial_event_count = fixture.canonical_work().events.len();
        let repo = fixture.git.repo.clone();
        let session_id = fixture.session.id.clone();
        let binding = fixture.binding.clone();
        let server = TerminalBridgeServer::start_with_hook(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
            move || {
                let receipt = crate::apply_bound_authenticated_work_terminalization(
                    &repo,
                    &session_id,
                    &binding,
                    crate::AgentWorkTerminalizationRequest {
                        schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                        claimed_session_id: session_id.clone(),
                        observation: crate::observe_agent_runtime(&repo)
                            .expect("observe exact Host emission"),
                        terminal_kind: crate::AgentWorkTerminalKind::Done,
                    },
                )
                .expect("emit exact canonical Host terminal");
                assert_eq!(
                    receipt.outcome,
                    crate::AgentWorkTerminalizationOutcome::Emitted
                );
            },
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete { spec: 3327 },
            )
        });

        assert!(
            result.is_ok(),
            "exact Host emission must confirm: {result:?}"
        );
        let work = fixture.canonical_work();
        assert!(work.is_terminal());
        assert!(!work.discarded);
        assert_eq!(work.events.len(), initial_event_count + 1);
        server.receive();
    }

    #[test]
    fn managed_build_rejects_work_reassignment_during_bridge_without_closing_old_work() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let old_work_before = fixture.canonical_work();
        let repo = fixture.git.repo.clone();
        let session_id = fixture.session.id.clone();
        let server = TerminalBridgeServer::start_with_hook(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
            move || {
                let mut current = gwt_core::workspace_projection::load_workspace_projection(&repo)
                    .expect("load current during reassignment")
                    .expect("current during reassignment");
                current
                    .agents
                    .iter_mut()
                    .find(|agent| agent.session_id == session_id)
                    .expect("assigned Session")
                    .workspace_id = Some("work-reassigned-during-bridge".to_string());
                gwt_core::workspace_projection::save_workspace_projection(&repo, &current)
                    .expect("save reassigned current");
            },
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete { spec: 3327 },
            )
        });

        assert!(result.is_err(), "changed Work id must fail closed");
        assert_eq!(
            fixture.canonical_work(),
            old_work_before,
            "the snapshotted Work must remain byte-equivalent in projection semantics"
        );
        assert_build_still_active(&fixture.git);
        server.receive();
    }

    #[test]
    fn managed_build_rejects_duplicate_canonical_session_authority_before_bridge() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let mut current =
            gwt_core::workspace_projection::load_workspace_projection(&fixture.git.repo)
                .expect("load canonical current")
                .expect("canonical current");
        let duplicate = current
            .agents
            .iter()
            .find(|agent| agent.session_id == fixture.session.id)
            .expect("canonical Session assignment")
            .clone();
        current.agents.push(duplicate);
        gwt_core::workspace_projection::save_workspace_projection(&fixture.git.repo, &current)
            .expect("seed duplicate Session assignment");
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let before = std::fs::read(&works_path).expect("snapshot canonical WorkItems");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete { spec: 3327 },
            )
        });

        assert!(result.is_err(), "duplicate authority must fail closed");
        assert_eq!(
            std::fs::read(&works_path).expect("read canonical WorkItems"),
            before
        );
        assert_build_still_active(&fixture.git);
        server.assert_no_request();
    }

    #[test]
    fn blocked_build_abort_duplicate_session_authority_guides_status_recovery_without_ensure_loop()
    {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();
        let mut current =
            gwt_core::workspace_projection::load_workspace_projection(&fixture.git.repo)
                .expect("load canonical current")
                .expect("canonical current");
        let duplicate = current
            .agents
            .iter()
            .find(|agent| agent.session_id == fixture.session.id)
            .expect("canonical Session assignment")
            .clone();
        current.agents.push(duplicate);
        gwt_core::workspace_projection::save_workspace_projection(&fixture.git.repo, &current)
            .expect("seed duplicate Session assignment");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
        );
        let repo = fixture.git.repo.clone();

        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("recover the stranded lifecycle".to_string()),
                },
                &mut output,
            )
            .expect("run blocked build.abort with ambiguous Work authority");
            (code, output)
        });

        assert_ne!(code, 0, "{output}");
        assert!(output.contains("execution.status"), "{output}");
        assert!(output.contains("recovery_probes"), "{output}");
        assert!(output.contains("verify.plan"), "{output}");
        assert!(
            !output.contains("run workspace.ensure"),
            "the refusal must not restart the build.abort/workspace.ensure loop: {output}"
        );
        let diagnosis =
            crate::cli::execution_state::diagnose(&fixture.git.repo, Some(&fixture.session.id));
        assert!(
            !diagnosis
                .available_recoveries
                .contains(&"build.abort".to_string()),
            "an operation rejected by the exact Work preflight must not be advertised: {diagnosis:?}"
        );
        let abort_probe = diagnosis
            .recovery_probes
            .iter()
            .find(|probe| probe.operation == "build.abort")
            .expect("build.abort operation-local recovery probe");
        assert_eq!(
            abort_probe.state,
            crate::cli::governance::RecoveryProbeState::Unavailable
        );
        assert_eq!(
            abort_probe.governance.cause,
            Some(crate::cli::governance::GovernanceCause::Authority)
        );
        assert_eq!(abort_probe.governance.retryable, Some(false));
        assert!(
            abort_probe
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("ambiguous")),
            "{abort_probe:?}"
        );
        assert_build_still_active(&fixture.git);
        server.assert_no_request();
    }

    #[test]
    fn blocked_build_abort_without_host_authority_guides_status_recovery() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();
        let mut docker = fixture.session.clone();
        docker.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        docker
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save Docker Session");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
        );
        let repo = fixture.git.repo.clone();

        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("recover without Host authority".to_string()),
                },
                &mut output,
            )
            .expect("run blocked build.abort without Host authority");
            (code, output)
        });

        assert_ne!(code, 0, "{output}");
        assert!(output.contains("execution.status"), "{output}");
        assert!(output.contains("available_recoveries"), "{output}");
        assert!(output.contains("verify.plan"), "{output}");
        let diagnosis =
            crate::cli::execution_state::diagnose(&fixture.git.repo, Some(&fixture.session.id));
        let abort_probe = diagnosis
            .recovery_probes
            .iter()
            .find(|probe| probe.operation == "build.abort")
            .expect("build.abort operation-local recovery probe");
        assert_eq!(
            abort_probe.governance.cause,
            Some(crate::cli::governance::GovernanceCause::Authority)
        );
        assert_eq!(abort_probe.governance.retryable, Some(false));
        assert_build_still_active(&fixture.git);
        server.assert_no_request();
    }

    #[test]
    fn managed_build_rejects_docker_authority_before_bridge() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let mut docker = fixture.session.clone();
        docker.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        docker
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save Docker Session");
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let before = std::fs::read(&works_path).expect("snapshot canonical WorkItems");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete { spec: 3327 },
            )
        });

        assert!(result.is_err(), "Docker authority must fail closed");
        assert_eq!(
            std::fs::read(&works_path).expect("read canonical WorkItems"),
            before
        );
        assert_build_still_active(&fixture.git);
        server.assert_no_request();
    }

    #[test]
    fn bridge_terminalization_requires_exact_durable_host_authority() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for managed in [false, true] {
            let server = TerminalBridgeServer::start(
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
            );
            let (code, output, fixture) = run_active_action(
                SkillStateAction::Complete { spec: 3327 },
                Some(&server.forward_url),
                Some("terminal-secret"),
                managed,
            );
            assert_eq!(code, 1, "managed={managed}: {output}");
            assert!(
                output.contains("exact durable Host Work authority"),
                "{output}"
            );
            assert_build_still_active(&fixture);
            server.assert_no_request();
        }
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
                "ambiguous-terminal",
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::AmbiguousTerminal),
            ),
        ] {
            let fixture = BoundTerminalFixture::new();
            let server = TerminalBridgeServer::start(status, body);
            let result = with_terminal_bridge_env(&fixture, &server, |env| {
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Complete { spec: 3327 },
                )
            });
            let output = result.expect_err("invalid Host result must fail closed");
            assert!(
                !output.contains("terminal-secret"),
                "{label}: Host response must not reflect the bearer into diagnostics: {output}"
            );
            assert_build_still_active(&fixture.git);
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
        let fixture = BoundTerminalFixture::new();
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &fixture.session.id);
        let _forward_url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &unavailable_url);
        let _forward_token =
            ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "terminal-secret");
        let _runtime = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            fixture.git.repo.join("managed-runtime.json"),
        );
        let env = crate::cli::TestEnv::new(fixture.git.repo.clone());
        let output = record_current_work_terminal_before_finalize(
            &env,
            &SkillStateAction::Complete { spec: 3327 },
        )
        .expect_err("transport failure must fail closed");
        assert!(
            output.contains("unavailable") && output.contains("outcome may be unknown"),
            "{output}"
        );
        assert_build_still_active(&fixture.git);
    }

    #[test]
    fn terminal_bridge_never_follows_redirects_or_forwards_its_bearer() {
        let server = TerminalBridgeServer::start_redirect();
        let target = crate::daemon_runtime::HookForwardTarget {
            url: server.forward_url.clone(),
            token: "redirect-secret".to_string(),
        };
        let request = crate::AgentWorkTerminalizationRequest {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            claimed_session_id: "redirect-session".to_string(),
            observation: crate::AgentRuntimeObservation {
                cwd: "/workspace/repo".to_string(),
                git_toplevel: "/workspace/repo".to_string(),
                repo_hash: "redirect-repo".to_string(),
                branch: "work/redirect".to_string(),
            },
            terminal_kind: crate::AgentWorkTerminalKind::Done,
        };

        let error =
            crate::daemon_runtime::send_work_terminalization_via_agent_bridge(&target, &request)
                .expect_err("redirected terminal bridge response must fail closed");

        assert!(!error.contains("redirect-secret"), "{error}");
        let (headers, _) = server.receive();
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer redirect-secret")
        );
        server.assert_no_redirect();
    }
}

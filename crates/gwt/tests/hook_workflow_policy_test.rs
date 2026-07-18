//! T-112 (SPEC #1935) — workflow-policy gating tests.

#[cfg(unix)]
use std::env;
use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use chrono::Utc;
use gwt::cli::{
    hook::{event_dispatcher, gwt_self_improvement_stop, workflow_policy, HookEvent, HookOutput},
    improvement::{
        candidate_public_values, ImprovementCaptureCommand, ImprovementCommand,
        ImprovementPromoteIssueCommand, ImprovementTypedEvidenceCommand,
    },
    CliCommand, DefaultCliEnv, TestEnv,
};
use gwt_agent::{session::GWT_SESSION_ID_ENV, AgentId, Session, GWT_SESSION_RUNTIME_PATH_ENV};
use gwt_core::process::hidden_command;
use gwt_core::{
    coordination::{
        post_entry, AuthorKind, BoardEntry, BoardEntryKind, BoardMention, BoardMentionTargetKind,
    },
    paths::gwt_sessions_dir,
    repo_hash::compute_repo_hash,
    test_support::{ScopedEnvVar, ScopedGwtHome},
    workspace_projection::{
        record_workspace_work_event, save_workspace_projection, WorkEvent, WorkEventKind,
        WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary, WorkspaceProjection,
        WorkspaceStatusCategory,
    },
};
use gwt_github::{
    client::{
        fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation},
        ApiError, IssueNumber, IssueSnapshot, IssueState, ResolutionDeadline, UpdatedAt,
    },
    Cache,
};
use serde_json::json;
use tempfile::TempDir;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

const DIRECT_STOP_TEST_TOTAL_BUDGET: Duration = Duration::from_millis(900);
const DIRECT_STOP_TEST_CONNECT_TIMEOUT: Duration = Duration::from_millis(150);
const DIRECT_STOP_TEST_SETTLEMENT_RESERVE: Duration = Duration::from_millis(250);

fn evaluate_direct_stop_with_test_budget(env: &mut DefaultCliEnv) -> HookOutput {
    let deadline = ResolutionDeadline::new(
        DIRECT_STOP_TEST_CONNECT_TIMEOUT,
        DIRECT_STOP_TEST_TOTAL_BUDGET,
    );
    gwt_self_improvement_stop::evaluate_with_deadline_and_reserve(
        env,
        false,
        false,
        &deadline,
        DIRECT_STOP_TEST_SETTLEMENT_RESERVE,
    )
}

fn root() -> PathBuf {
    std::env::temp_dir().join("gwt-test-worktree")
}

fn outside_root() -> PathBuf {
    root()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("gwt-test-outside")
}

fn event(tool_name: &str, tool_input: serde_json::Value) -> HookEvent {
    serde_json::from_value(json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
    }))
    .expect("valid hook event")
}

fn json_envelope_command(operation: &str, params: serde_json::Value) -> String {
    let body = json!({
        "schema_version": 1,
        "operation": operation,
        "params": params,
    });
    format!("gwtd <<'JSON'\n{}\nJSON", body)
}

fn evaluate(event: &HookEvent, context: workflow_policy::WorkflowContext) -> Option<HookOutput> {
    match workflow_policy::evaluate_with_context(event, Path::new(&root()), &context)
        .expect("evaluation should succeed")
    {
        HookOutput::Silent => None,
        other => Some(other),
    }
}

fn with_temp_home<T>(f: impl FnOnce(&TempDir) -> T) -> T {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("temp home");
    let _home = ScopedGwtHome::set(home.path());
    let _session_id = ScopedEnvVar::unset(GWT_SESSION_ID_ENV);

    f(&home)
}

fn write_improvement_store(repo_path: &Path, candidates: serde_json::Value) {
    let path = gwt_core::paths::gwt_project_dir_for_repo_path(repo_path)
        .join("improvements")
        .join("candidates.json");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create improvements dir");
    std::fs::write(path, candidates.to_string()).expect("write candidates");
}

fn init_repo_with_origin(remote_url: &str) -> TempDir {
    let repo = tempfile::tempdir().expect("repo");
    assert!(hidden_command("git")
        .arg("init")
        .arg("-q")
        .arg(repo.path())
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(repo.path())
        .args(["remote", "add", "origin", remote_url])
        .status()
        .expect("git remote add")
        .success());
    repo
}

fn self_improvement_test_env(repo_path: &Path) -> TestEnv {
    let mut env = TestEnv::new(repo_path.join("cache"));
    env.repo_path = repo_path.to_path_buf();
    env
}

fn save_verified_improvement_session(repo_path: &Path) -> String {
    let mut session = Session::new(repo_path, "test", AgentId::Codex);
    session.repo_hash = Some(gwt_core::paths::project_scope_hash(repo_path).to_string());
    let id = session.id.clone();
    session
        .save(&gwt_sessions_dir())
        .expect("save verified improvement session");
    id
}

fn typed_improvement_capture(failure_code: &str) -> CliCommand {
    CliCommand::Improvement(ImprovementCommand::Capture(Box::new(
        ImprovementCaptureCommand {
            source: "hook-runtime".to_string(),
            target_artifact: "coordination".to_string(),
            classification: "gwt-caused".to_string(),
            confidence: "high".to_string(),
            summary: "Direct Stop must bound Owner Resolution".to_string(),
            details: None,
            evidence_digest: None,
            dedupe_key: None,
            local_evidence: Vec::new(),
            typed_evidence: Some(ImprovementTypedEvidenceCommand {
                subsystem: "coordination".to_string(),
                contract_id: "coordination.board-status".to_string(),
                contract_schema_revision: 1,
                failure_code: failure_code.to_string(),
                expected_outcome: "BOARD_STATUS_POSTED".to_string(),
                observed_outcome: "BOARD_STATUS_MISSING".to_string(),
            }),
        },
    )))
}

fn capture_interpretive_candidate(env: &mut TestEnv, session_id: &str, failure_code: &str) {
    std::env::set_var(GWT_SESSION_ID_ENV, session_id);
    gwt::cli::run(env, typed_improvement_capture(failure_code)).expect("typed capture");
}

fn sync_test_env_source_scope_nonce(env: &mut TestEnv) {
    let path = gwt_core::paths::gwt_project_dir_for_repo_path(&env.repo_path)
        .join("improvements")
        .join("candidates.json");
    let store: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path).expect("read candidate store for test env nonce"),
    )
    .expect("parse candidate store for test env nonce");
    env.improvement_source_scope_nonce = store["source_scope_nonce"]
        .as_str()
        .expect("candidate store source scope nonce")
        .to_string();
}

fn fail_next_owner_corpus_with_timeout(env: &TestEnv) {
    env.owner_client.fail_next_owner_operation(
        OwnerRepositoryOperation::ListIssues,
        OwnerRepositoryFaultTiming::BeforeSubmit,
        ApiError::Timeout {
            operation: "stalled owner corpus".to_string(),
        },
    );
}

fn read_http_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("request read timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request headers");
        assert!(read > 0, "request closed before headers");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .unwrap_or(0);
    while bytes.len() - header_end < content_length {
        let read = stream.read(&mut chunk).expect("read request body");
        assert!(read > 0, "request closed before body");
        bytes.extend_from_slice(&chunk[..read]);
    }
}

fn accept_loopback_with_timeout(listener: &TcpListener, timeout: Duration) -> TcpStream {
    listener
        .set_nonblocking(true)
        .expect("nonblocking loopback listener");
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("loopback accept failed before {timeout:?}: {error}"),
        }
    }
}

fn only_candidate_id(repo_path: &Path) -> String {
    let candidates = candidate_public_values(repo_path);
    assert_eq!(candidates.len(), 1, "expected one deduplicated candidate");
    candidates[0]["id"]
        .as_str()
        .expect("candidate id")
        .to_string()
}

#[test]
fn gwt_self_improvement_stop_retries_one_candidate_with_the_strict_budget() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let session_a = save_verified_improvement_session(repo.path());
        let session_b = save_verified_improvement_session(repo.path());
        let mut env = TestEnv::new(repo.path().join("cache"));
        env.repo_path = repo.path().to_path_buf();

        capture_interpretive_candidate(&mut env, &session_a, "STATUS_NOT_POSTED");
        sync_test_env_source_scope_nonce(&mut env);
        fail_next_owner_corpus_with_timeout(&env);
        capture_interpretive_candidate(&mut env, &session_b, "STATUS_NOT_POSTED");
        let candidate_id = only_candidate_id(repo.path());

        assert_eq!(env.owner_client_access_count(), 1);
        let (normal_connect, normal_total) = env
            .last_owner_client_budget()
            .expect("normal capture owner budget");
        assert!(normal_connect <= Duration::from_secs(5));
        assert!(normal_total <= Duration::from_secs(120));
        assert!(normal_total > Duration::from_secs(110));
        assert_eq!(
            env.last_owner_client_access_saw_persisted_candidate(),
            Some(true),
            "capture must persist the owner-resolving state before client access"
        );
        assert_eq!(
            env.last_owner_client_candidate_id().as_deref(),
            Some(candidate_id.as_str())
        );

        env.clear_owner_client_access_log();
        fail_next_owner_corpus_with_timeout(&env);
        gwt::cli::run(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: candidate_id.clone(),
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect("explicit Owner Resolution retry");
        assert_eq!(env.owner_client_access_count(), 1);
        let (explicit_connect, explicit_total) = env
            .last_owner_client_budget()
            .expect("explicit resolve owner budget");
        assert!(explicit_connect <= Duration::from_secs(5));
        assert!(explicit_total <= Duration::from_secs(120));
        assert!(explicit_total > Duration::from_secs(110));

        env.clear_owner_client_access_log();
        fail_next_owner_corpus_with_timeout(&env);
        let output = gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false);

        let HookOutput::StopBlock { reason } = output else {
            panic!("unresolved strict Stop resolution must block");
        };
        assert!(reason.contains("state=blocked"), "{reason}");
        assert!(reason.contains("reason=timeout"), "{reason}");
        assert!(reason.contains("RETRY_WITHIN_BUDGET"), "{reason}");
        assert_eq!(env.owner_client_access_count(), 1);
        let (strict_connect, strict_total) = env
            .last_owner_client_budget()
            .expect("strict Stop owner budget");
        assert!(strict_connect <= Duration::from_secs(3));
        assert!(strict_total <= Duration::from_secs(15));
        assert!(strict_total > Duration::from_secs(10));
        assert_eq!(
            env.last_owner_client_candidate_id().as_deref(),
            Some(candidate_id.as_str())
        );
    });
}

#[cfg(unix)]
#[test]
fn direct_stop_retries_pending_owner_status_without_rerunning_owner_resolution() {
    use std::os::unix::fs::PermissionsExt;

    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let session_a = save_verified_improvement_session(repo.path());
        let session_b = save_verified_improvement_session(repo.path());
        let mut env = TestEnv::new(repo.path().join("cache"));
        env.repo_path = repo.path().to_path_buf();

        capture_interpretive_candidate(&mut env, &session_a, "STATUS_NOT_POSTED");
        sync_test_env_source_scope_nonce(&mut env);
        let fingerprint = candidate_public_values(repo.path())[0]["fingerprint"]
            .as_str()
            .expect("candidate fingerprint")
            .to_string();
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: IssueNumber(77),
                title: "Existing self-improvement owner".to_string(),
                body: format!("<!-- gwt:improvement-fingerprint:v1 {fingerprint} -->"),
                labels: Vec::new(),
                state: IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: UpdatedAt::new("owner-77"),
            });
        capture_interpretive_candidate(&mut env, &session_b, "STATUS_NOT_POSTED");

        let store_path = gwt_core::paths::gwt_project_dir_for_repo_path(repo.path())
            .join("improvements")
            .join("candidates.json");
        let mut store: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&store_path).expect("read created candidate store"),
        )
        .expect("parse created candidate store");
        let candidate = &mut store["candidates"][0];
        assert_eq!(candidate["state"], "linked");
        assert_eq!(candidate["owner_status_generation"], 1);
        assert_eq!(candidate["owner_status_delivered_generation"], 1);
        candidate["owner_status_delivered_generation"] = json!(0);
        std::fs::write(
            &store_path,
            serde_json::to_vec_pretty(&store).expect("serialize pending owner status"),
        )
        .expect("persist pending owner status");
        let create_calls_before = env
            .owner_client
            .owner_mutation_call_log()
            .iter()
            .filter(|call| call.operation == OwnerRepositoryOperation::CreateIssue)
            .count();
        env.clear_owner_client_access_log();

        let manifest_path = gwt_core::coordination::coordination_events_manifest_path(repo.path());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&manifest_path).expect("read Board event manifest"),
        )
        .expect("parse Board event manifest");
        let active_segment = gwt_core::coordination::coordination_events_segments_dir(repo.path())
            .join(
                manifest["active_segment"]
                    .as_str()
                    .expect("active Board event segment"),
            );
        std::fs::set_permissions(&active_segment, std::fs::Permissions::from_mode(0o400))
            .expect("make Board event segment read-only");

        let blocked = gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false);

        std::fs::set_permissions(&active_segment, std::fs::Permissions::from_mode(0o600))
            .expect("restore Board event segment permissions");
        let HookOutput::StopBlock { reason } = blocked else {
            panic!("failed owner status delivery must block Stop");
        };
        assert!(reason.contains("state=linked"), "{reason}");
        assert!(reason.contains("reason=status-delivery"), "{reason}");
        assert!(
            reason.contains("remediation=RETRY_OWNER_STATUS"),
            "{reason}"
        );
        let still_pending: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&store_path).expect("read pending candidate store"),
        )
        .expect("parse pending candidate store");
        assert_eq!(
            still_pending["candidates"][0]["owner_status_delivered_generation"],
            0
        );
        assert_eq!(env.owner_client_access_count(), 0);

        let output = gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false);

        assert_eq!(output, HookOutput::Silent);
        assert_eq!(
            env.owner_client_access_count(),
            0,
            "status delivery retry must not rerun Owner Resolution"
        );
        let persisted: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&store_path).expect("read acknowledged candidate store"),
        )
        .expect("parse acknowledged candidate store");
        assert_eq!(
            persisted["candidates"][0]["owner_status_delivered_generation"],
            persisted["candidates"][0]["owner_status_generation"]
        );
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::CreateIssue)
                .count(),
            create_calls_before,
            "status retry must not create another owner"
        );
    });
}

#[test]
fn direct_stop_composes_pagination_and_transport_stall_within_strict_budget() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let session_a = save_verified_improvement_session(repo.path());
        let session_b = save_verified_improvement_session(repo.path());
        let mut seed_env = TestEnv::new(repo.path().join("cache"));
        seed_env.repo_path = repo.path().to_path_buf();
        capture_interpretive_candidate(&mut seed_env, &session_a, "STATUS_NOT_POSTED");
        sync_test_env_source_scope_nonce(&mut seed_env);
        fail_next_owner_corpus_with_timeout(&seed_env);
        capture_interpretive_candidate(&mut seed_env, &session_b, "STATUS_NOT_POSTED");

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let address = listener.local_addr().expect("loopback address");
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = Arc::clone(&requests);
        let server = std::thread::spawn(move || {
            let mut first = accept_loopback_with_timeout(&listener, Duration::from_secs(2));
            server_requests.fetch_add(1, Ordering::SeqCst);
            read_http_request(&mut first);
            let body = r#"{"data":{"repository":{"issues":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"cursor-1"}}}}}"#;
            write!(
                first,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("first page response");
            first.flush().expect("flush first page");
            drop(first);

            let mut stalled = accept_loopback_with_timeout(&listener, Duration::from_secs(2));
            server_requests.fetch_add(1, Ordering::SeqCst);
            read_http_request(&mut stalled);
            stalled
                .set_read_timeout(Some(Duration::from_millis(100)))
                .expect("stall read timeout");
            let until = Instant::now() + Duration::from_millis(850);
            let mut byte = [0_u8; 1];
            while Instant::now() < until {
                match stalled.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error)
                        if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                    Err(_) => break,
                }
            }
        });
        let _mode = ScopedEnvVar::set("GWT_OWNER_GITHUB_TEST_MODE", "loopback-v1");
        let _rest = ScopedEnvVar::set("GWT_OWNER_GITHUB_REST_BASE", format!("http://{address}"));
        let _graphql = ScopedEnvVar::set(
            "GWT_OWNER_GITHUB_GRAPHQL_URL",
            format!("http://{address}/graphql"),
        );
        let _token = ScopedEnvVar::set("GWT_OWNER_GITHUB_TOKEN", "loopback-test-token");
        let mut env = DefaultCliEnv::new_for_hooks_at(repo.path().to_path_buf());
        let started = Instant::now();

        let output = evaluate_direct_stop_with_test_budget(&mut env);
        let elapsed = started.elapsed();

        let HookOutput::StopBlock { reason } = output else {
            panic!("stalled direct Stop owner search must block");
        };
        assert!(reason.contains("reason=timeout"), "{reason}");
        assert!(
            elapsed < DIRECT_STOP_TEST_TOTAL_BUDGET + Duration::from_secs(1),
            "elapsed={elapsed:?}"
        );
        let persisted = candidate_public_values(repo.path());
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0]["state"], "blocked");
        assert_eq!(persisted[0]["blocked_reason"], "timeout");
        let store_path = gwt_core::paths::gwt_project_dir_for_repo_path(repo.path())
            .join("improvements")
            .join("candidates.json");
        let store: serde_json::Value = serde_json::from_slice(
            &std::fs::read(store_path).expect("read durable candidate store"),
        )
        .expect("parse durable candidate store");
        assert!(store["candidates"][0]["attempt"].is_null());
        server.join().expect("loopback server");
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn direct_stop_reserves_time_to_settle_after_post_attempt_store_lock_contention() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let session_a = save_verified_improvement_session(repo.path());
        let session_b = save_verified_improvement_session(repo.path());
        let mut seed_env = TestEnv::new(repo.path().join("cache"));
        seed_env.repo_path = repo.path().to_path_buf();
        capture_interpretive_candidate(&mut seed_env, &session_a, "STATUS_NOT_POSTED");
        sync_test_env_source_scope_nonce(&mut seed_env);
        fail_next_owner_corpus_with_timeout(&seed_env);
        capture_interpretive_candidate(&mut seed_env, &session_b, "STATUS_NOT_POSTED");

        let candidate_lock_path = gwt_core::paths::gwt_project_dir_for_repo_path(repo.path())
            .join("improvements")
            .join(".lock");
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let address = listener.local_addr().expect("loopback address");
        let server = std::thread::spawn(move || {
            let mut request = accept_loopback_with_timeout(&listener, Duration::from_secs(2));
            read_http_request(&mut request);
            let candidate_lock = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(candidate_lock_path)
                .expect("candidate store lock");
            fs2::FileExt::lock_exclusive(&candidate_lock).expect("hold candidate store lock");
            let body = r#"{"data":{"repository":{"issues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#;
            write!(
                request,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("complete owner corpus response");
            request.flush().expect("flush owner corpus response");
            std::thread::sleep(Duration::from_millis(700));
            fs2::FileExt::unlock(&candidate_lock).expect("release candidate store lock");
        });
        let _mode = ScopedEnvVar::set("GWT_OWNER_GITHUB_TEST_MODE", "loopback-v1");
        let _rest = ScopedEnvVar::set("GWT_OWNER_GITHUB_REST_BASE", format!("http://{address}"));
        let _graphql = ScopedEnvVar::set(
            "GWT_OWNER_GITHUB_GRAPHQL_URL",
            format!("http://{address}/graphql"),
        );
        let _token = ScopedEnvVar::set("GWT_OWNER_GITHUB_TOKEN", "loopback-test-token");
        let mut env = DefaultCliEnv::new_for_hooks_at(repo.path().to_path_buf());
        let started = Instant::now();

        let output = evaluate_direct_stop_with_test_budget(&mut env);
        let elapsed = started.elapsed();

        server.join().expect("loopback server");
        let HookOutput::StopBlock { reason } = output else {
            panic!("post-attempt store contention must block Stop");
        };
        assert!(reason.contains("state=blocked"), "{reason}");
        assert!(reason.contains("reason=timeout"), "{reason}");
        assert!(reason.contains("RETRY_WITHIN_BUDGET"), "{reason}");
        assert!(
            elapsed < DIRECT_STOP_TEST_TOTAL_BUDGET + Duration::from_secs(1),
            "elapsed={elapsed:?}"
        );
        let persisted = candidate_public_values(repo.path());
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0]["state"], "blocked");
        assert_eq!(persisted[0]["blocked_reason"], "timeout");
        let store_path = gwt_core::paths::gwt_project_dir_for_repo_path(repo.path())
            .join("improvements")
            .join("candidates.json");
        let store: serde_json::Value = serde_json::from_slice(
            &std::fs::read(store_path).expect("read settled candidate store"),
        )
        .expect("parse settled candidate store");
        assert!(store["candidates"][0]["attempt"].is_null());
    });
}

#[cfg(unix)]
#[test]
fn direct_stop_terminates_stalled_lazy_auth_and_persists_timeout() {
    use std::os::unix::fs::PermissionsExt;

    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let session_a = save_verified_improvement_session(repo.path());
        let session_b = save_verified_improvement_session(repo.path());
        let mut seed_env = TestEnv::new(repo.path().join("cache"));
        seed_env.repo_path = repo.path().to_path_buf();
        capture_interpretive_candidate(&mut seed_env, &session_a, "STATUS_NOT_POSTED");
        sync_test_env_source_scope_nonce(&mut seed_env);
        fail_next_owner_corpus_with_timeout(&seed_env);
        capture_interpretive_candidate(&mut seed_env, &session_b, "STATUS_NOT_POSTED");

        let fake_bin = tempfile::tempdir().expect("fake bin");
        let fake_gh = fake_bin.path().join("gh");
        std::fs::write(&fake_gh, "#!/bin/sh\nsleep 0.8\nprintf 'late-token\\n'\n")
            .expect("write fake gh");
        std::fs::set_permissions(&fake_gh, std::fs::Permissions::from_mode(0o755))
            .expect("make fake gh executable");
        let path = env::join_paths(
            std::iter::once(fake_bin.path().to_path_buf())
                .chain(env::split_paths(&env::var_os("PATH").expect("PATH"))),
        )
        .expect("compose PATH");
        let _path = ScopedEnvVar::set("PATH", path);
        let _mode = ScopedEnvVar::unset("GWT_OWNER_GITHUB_TEST_MODE");
        let _rest = ScopedEnvVar::unset("GWT_OWNER_GITHUB_REST_BASE");
        let _graphql = ScopedEnvVar::unset("GWT_OWNER_GITHUB_GRAPHQL_URL");
        let _token = ScopedEnvVar::unset("GWT_OWNER_GITHUB_TOKEN");
        let mut env = DefaultCliEnv::new_for_hooks_at(repo.path().to_path_buf());
        let started = Instant::now();

        let output = evaluate_direct_stop_with_test_budget(&mut env);
        let elapsed = started.elapsed();

        let HookOutput::StopBlock { reason } = output else {
            panic!("stalled direct Stop auth must block");
        };
        assert!(reason.contains("reason=timeout"), "{reason}");
        assert!(
            elapsed < DIRECT_STOP_TEST_TOTAL_BUDGET + Duration::from_secs(1),
            "elapsed={elapsed:?}"
        );
        let persisted = candidate_public_values(repo.path());
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0]["state"], "blocked");
        assert_eq!(persisted[0]["blocked_reason"], "timeout");
        let store_path = gwt_core::paths::gwt_project_dir_for_repo_path(repo.path())
            .join("improvements")
            .join("candidates.json");
        let store: serde_json::Value = serde_json::from_slice(
            &std::fs::read(store_path).expect("read durable candidate store"),
        )
        .expect("parse durable candidate store");
        assert!(store["candidates"][0]["attempt"].is_null());
    });
}

#[test]
fn direct_self_improvement_hook_dispatch_uses_the_injected_owner_client() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let session_a = save_verified_improvement_session(repo.path());
        let session_b = save_verified_improvement_session(repo.path());
        let mut env = TestEnv::new(repo.path().join("cache"));
        env.repo_path = repo.path().to_path_buf();

        capture_interpretive_candidate(&mut env, &session_a, "STATUS_NOT_POSTED");
        sync_test_env_source_scope_nonce(&mut env);
        fail_next_owner_corpus_with_timeout(&env);
        capture_interpretive_candidate(&mut env, &session_b, "STATUS_NOT_POSTED");

        env.stdout.clear();
        env.clear_owner_client_access_log();
        env.stdin = "{}".to_string();
        fail_next_owner_corpus_with_timeout(&env);
        let code = gwt::cli::hook::run_hook(&mut env, "gwt-self-improvement-stop", &[])
            .expect("direct Stop hook dispatch");

        assert_eq!(code, 0);
        let output = String::from_utf8(env.stdout.clone()).expect("hook stdout");
        assert!(output.contains("reason=timeout"), "{output}");
        assert_eq!(env.owner_client_access_count(), 1);
    });
}

#[test]
fn gwt_self_improvement_stop_attempts_at_most_one_unresolved_candidate() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let session_a = save_verified_improvement_session(repo.path());
        let session_b = save_verified_improvement_session(repo.path());
        let mut env = TestEnv::new(repo.path().join("cache"));
        env.repo_path = repo.path().to_path_buf();

        for failure_code in ["STATUS_NOT_POSTED", "SECOND_STATUS_NOT_POSTED"] {
            capture_interpretive_candidate(&mut env, &session_a, failure_code);
            sync_test_env_source_scope_nonce(&mut env);
            fail_next_owner_corpus_with_timeout(&env);
            capture_interpretive_candidate(&mut env, &session_b, failure_code);
        }

        env.clear_owner_client_access_log();
        fail_next_owner_corpus_with_timeout(&env);
        let output = gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false);
        let first_attempted = env
            .last_owner_client_candidate_id()
            .expect("first attempted candidate id");

        let HookOutput::StopBlock { reason } = output else {
            panic!("remaining unresolved candidates must block");
        };
        assert_eq!(
            env.owner_client_access_count(),
            1,
            "one Stop invocation may start only one Owner Resolution attempt"
        );
        assert_eq!(
            reason.matches("state=blocked").count(),
            2,
            "the blocker must report every unresolved candidate after the single attempt: {reason}"
        );

        env.clear_owner_client_access_log();
        fail_next_owner_corpus_with_timeout(&env);
        let second_output = gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false);
        let second_attempted = env
            .last_owner_client_candidate_id()
            .expect("second attempted candidate id");
        assert_ne!(
            first_attempted, second_attempted,
            "repeated Stop attempts must not starve older unresolved candidates"
        );
        assert!(matches!(second_output, HookOutput::StopBlock { .. }));
    });
}

#[test]
fn gwt_self_improvement_stop_fails_closed_for_a_corrupt_candidate_store() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let mut env = TestEnv::new(repo.path().join("cache"));
        env.repo_path = repo.path().to_path_buf();
        let store_path = gwt_core::paths::gwt_project_dir_for_repo_path(repo.path())
            .join("improvements")
            .join("candidates.json");
        std::fs::create_dir_all(store_path.parent().expect("candidate store parent"))
            .expect("create candidate store parent");
        std::fs::write(&store_path, b"{not-json").expect("write corrupt candidate store");

        let output = gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false);

        let HookOutput::StopBlock { reason } = output else {
            panic!("a corrupt gwt candidate store must fail closed");
        };
        assert!(reason.contains("reason=store"), "{reason}");
        assert!(reason.contains("REPAIR_CANDIDATE_STORE"), "{reason}");
        assert_eq!(env.owner_client_access_count(), 0);
    });
}

#[test]
fn gwt_self_improvement_stop_fails_closed_when_repo_probe_cannot_run() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        let mut env = self_improvement_test_env(repo.path());
        let empty_path = tempfile::tempdir().expect("empty PATH directory");
        let _path = ScopedEnvVar::set("PATH", empty_path.path());

        let output = gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false);

        let HookOutput::StopBlock { reason } = output else {
            panic!("a failed gwt repository probe must fail closed");
        };
        assert!(reason.contains("reason=routing"), "{reason}");
        assert!(reason.contains("RETRY_REPOSITORY_PROBE"), "{reason}");
        assert_eq!(env.owner_client_access_count(), 0);
    });
}

#[test]
fn gwt_self_improvement_stop_ignores_legacy_high_confidence_candidate_without_typed_evidence() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        write_improvement_store(
            repo.path(),
            json!({
            "candidates": [{
                "id": "impr-high",
                "created_at": "2026-06-23T00:00:00Z",
                "updated_at": "2026-06-23T00:00:00Z",
                "source": "agent-failure",
                "target_artifact": "skill",
                "classification": "gwt-caused",
                "confidence": "high",
                "state": "pending",
                "dedupe_key": "skill:gwt-discussion:self-improvement",
                "occurrences": 1,
                "sanitized_summary": "Skill failed to update after agent failure",
                "sanitized_details": "Public-safe detail",
                "evidence_digest": "Public-safe digest",
                "local_evidence": [],
                "linked_issue": null,
                "dismissed_reason": null
            }]
            }),
        );
        let mut env = self_improvement_test_env(repo.path());

        assert_eq!(
            gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false),
            HookOutput::Silent,
            "legacy free-form evidence must migrate to needs-evidence without blocking Stop"
        );

        // SPEC-3247 FR-003 / AS-4: the same high-confidence candidate in an intake
        // (Curate) session must NOT block Stop — intake owns no Work and is not the
        // producing-work self-improvement loop.
        assert_eq!(
            gwt_self_improvement_stop::evaluate_with_env(&mut env, false, true),
            HookOutput::Silent,
            "intake sessions must not be forced to handle improvement candidates"
        );
        assert_eq!(env.owner_client_access_count(), 0);
    });
}

#[test]
fn gwt_self_improvement_stop_ignores_low_confidence_or_handled_candidates() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("git@github.com:akiojin/gwt.git");
        write_improvement_store(
            repo.path(),
            json!({
            "candidates": [
                {
                    "id": "impr-low",
                    "created_at": "2026-06-23T00:00:00Z",
                    "updated_at": "2026-06-23T00:00:00Z",
                    "source": "agent-failure",
                    "target_artifact": "skill",
                    "classification": "gwt-caused",
                    "confidence": "low",
                    "state": "pending",
                    "dedupe_key": "skill:low",
                    "occurrences": 1,
                    "sanitized_summary": "Low confidence",
                    "sanitized_details": null,
                    "evidence_digest": null,
                    "local_evidence": [],
                    "linked_issue": null,
                    "dismissed_reason": null
                },
                {
                    "id": "impr-promoted",
                    "created_at": "2026-06-23T00:00:00Z",
                    "updated_at": "2026-06-23T00:00:00Z",
                    "source": "agent-failure",
                    "target_artifact": "skill",
                    "classification": "gwt-caused",
                    "confidence": "high",
                    "state": "promoted",
                    "dedupe_key": "skill:promoted",
                    "occurrences": 1,
                    "sanitized_summary": "Already promoted",
                    "sanitized_details": null,
                    "evidence_digest": null,
                    "local_evidence": [],
                    "linked_issue": {"number": 1, "url": "https://github.com/akiojin/gwt/issues/1", "repository": "akiojin/gwt"},
                    "dismissed_reason": null
                }
            ]
            }),
        );
        let mut env = self_improvement_test_env(repo.path());

        assert_eq!(
            gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false),
            HookOutput::Silent
        );
        assert_eq!(env.owner_client_access_count(), 0);
    });
}

#[test]
fn gwt_self_improvement_stop_is_noop_outside_gwt_repo() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/example/target-project.git");
        write_improvement_store(
            repo.path(),
            json!({
            "candidates": [{
                "id": "impr-target",
                "created_at": "2026-06-23T00:00:00Z",
                "updated_at": "2026-06-23T00:00:00Z",
                "source": "agent-failure",
                "target_artifact": "skill",
                "classification": "gwt-caused",
                "confidence": "high",
                "state": "pending",
                "dedupe_key": "target:skill",
                "occurrences": 1,
                "sanitized_summary": "Target project saw a gwt hook problem",
                "sanitized_details": null,
                "evidence_digest": null,
                "local_evidence": [],
                "linked_issue": null,
                "dismissed_reason": null
            }]
            }),
        );
        let mut env = self_improvement_test_env(repo.path());

        assert_eq!(
            gwt_self_improvement_stop::evaluate_with_env(&mut env, false, false),
            HookOutput::Silent
        );
        assert_eq!(env.owner_client_access_count(), 0);
    });
}

#[test]
fn gwt_self_improvement_stop_respects_stop_hook_active() {
    with_temp_home(|_| {
        let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
        write_improvement_store(
            repo.path(),
            json!({
            "candidates": [{
                "id": "impr-active-stop",
                "created_at": "2026-06-23T00:00:00Z",
                "updated_at": "2026-06-23T00:00:00Z",
                "source": "agent-failure",
                "target_artifact": "hook",
                "classification": "gwt-caused",
                "confidence": "high",
                "state": "pending",
                "dedupe_key": "hook:active-stop",
                "occurrences": 1,
                "sanitized_summary": "Stop hook recursion must not block again",
                "sanitized_details": null,
                "evidence_digest": null,
                "local_evidence": [],
                "linked_issue": null,
                "dismissed_reason": null
            }]
            }),
        );
        let mut env = self_improvement_test_env(repo.path());

        assert_eq!(
            gwt_self_improvement_stop::evaluate_with_env(&mut env, true, false),
            HookOutput::Silent
        );
        assert_eq!(env.owner_client_access_count(), 0);
    });
}

#[test]
fn common_stop_dispatcher_does_not_run_gwt_self_improvement_stop() {
    let _env_guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _session_id = ScopedEnvVar::unset(GWT_SESSION_ID_ENV);
    let _runtime_path = ScopedEnvVar::unset(GWT_SESSION_RUNTIME_PATH_ENV);
    let home = tempfile::tempdir().expect("temp home");
    let _home = ScopedGwtHome::set(home.path());
    let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
    write_improvement_store(
        repo.path(),
        json!({
            "candidates": [{
                "id": "impr-common-stop",
                "created_at": "2026-06-23T00:00:00Z",
                "updated_at": "2026-06-23T00:00:00Z",
                "source": "agent-failure",
                "target_artifact": "hook",
                "classification": "gwt-caused",
                "confidence": "high",
                "state": "pending",
                "dedupe_key": "hook:common-stop",
                "occurrences": 1,
                "sanitized_summary": "Common Stop dispatcher must not own self-improvement",
                "sanitized_details": null,
                "evidence_digest": null,
                "local_evidence": [],
                "linked_issue": null,
                "dismissed_reason": null
            }]
        }),
    );

    let output = event_dispatcher::handle_with_input("Stop", "{}", repo.path(), None)
        .expect("Stop dispatch");
    assert!(
        !matches!(output, HookOutput::StopBlock { .. }),
        "self-improvement must be invoked by direct gwt repo hook config, not common Stop dispatcher: {output:?}"
    );
}

fn init_repo(home: &TempDir) -> PathBuf {
    let repo_path = home.path().join("repo");
    std::fs::create_dir_all(&repo_path).expect("create repo dir");
    assert!(hidden_command("git")
        .arg("init")
        .arg(&repo_path)
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(&repo_path)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/gwt-test.git"
        ])
        .status()
        .expect("git remote add")
        .success());
    repo_path
}

fn seed_issue_cache(
    repo_path: &Path,
    issue_number: u64,
    labels: Vec<&str>,
    plan: &str,
    tasks: &str,
) {
    let repo_hash = compute_repo_hash("https://github.com/example/gwt-test.git");
    let cache_root = repo_path
        .parent()
        .expect("repo parent")
        .join(".gwt/cache/issues")
        .join(repo_hash.as_str());
    let cache = Cache::new(cache_root);
    let body = format!(
        "<!-- gwt-spec id={issue_number} version=1 -->\n\
<!-- sections:\n\
spec=body\n\
plan=body\n\
tasks=body\n\
-->\n\
<!-- artifact:spec BEGIN -->\n\
Workflow policy\n\
<!-- artifact:spec END -->\n\
<!-- artifact:plan BEGIN -->\n\
{plan}\n\
<!-- artifact:plan END -->\n\
<!-- artifact:tasks BEGIN -->\n\
{tasks}\n\
<!-- artifact:tasks END -->\n"
    );
    cache
        .write_snapshot(&IssueSnapshot {
            number: IssueNumber(issue_number),
            title: format!("Issue {issue_number}"),
            body,
            labels: labels.into_iter().map(str::to_string).collect(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-04-13T00:00:00Z"),
            comments: vec![],
        })
        .expect("seed issue cache");
}

fn save_session(repo_path: &Path, branch: &str, linked_issue_number: Option<u64>) -> String {
    let mut session = Session::new(repo_path, branch, AgentId::Codex);
    session.id = "session-workflow-policy".to_string();
    session.linked_issue_number = linked_issue_number;
    session.save(&gwt_sessions_dir()).expect("save session");
    session.id
}

fn seed_workspace_agent_title(repo_path: &Path, session_id: &str) {
    let mut projection = WorkspaceProjection::default_for_project(repo_path);
    projection.agents.push(workspace_agent(
        session_id,
        "Testing workflow policy",
        "Workflow policy test",
    ));
    save_workspace_projection(repo_path, &projection).expect("save workspace projection");
}

fn workspace_agent(
    session_id: &str,
    current_focus: &str,
    title_summary: &str,
) -> WorkspaceAgentSummary {
    WorkspaceAgentSummary {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some(current_focus.to_string()),
        title_summary: Some(title_summary.to_string()),
        worktree_path: None,
        branch: Some("feature/workflow".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("workspace-existing".to_string()),
        updated_at: Utc::now(),
    }
}

fn unassigned_workspace_agent(session_id: &str) -> WorkspaceAgentSummary {
    WorkspaceAgentSummary {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: None,
        branch: Some("work/unassigned".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: Utc::now(),
    }
}

fn seed_workspace_agents(
    repo_path: &Path,
    current_session_id: &str,
    current_title: &str,
    other_session_id: &str,
    other_title: &str,
) {
    let mut projection = WorkspaceProjection::default_for_project(repo_path);
    projection.title = "Workspace semantic coordination".to_string();
    projection.status_category = WorkspaceStatusCategory::Active;
    projection.summary = Some("Coordinate same-work detection across agents".to_string());
    projection.agents.push(workspace_agent(
        current_session_id,
        "Implement Workspace semantic coordination gate",
        current_title,
    ));
    projection.agents.push(workspace_agent(
        other_session_id,
        "Implement duplicate Workspace semantic coordination protection",
        other_title,
    ));
    save_workspace_projection(repo_path, &projection).expect("save workspace projection");
}

fn seed_workspace_current_agent(repo_path: &Path, session_id: &str, title: &str, focus: &str) {
    let mut projection = WorkspaceProjection::default_for_project(repo_path);
    projection
        .agents
        .push(workspace_agent(session_id, focus, title));
    save_workspace_projection(repo_path, &projection).expect("save workspace projection");
}

fn seed_workspace_work_item(
    repo_path: &Path,
    work_item_id: &str,
    kind: WorkEventKind,
    title: &str,
    session_id: &str,
) {
    let mut event = WorkEvent::new(kind, work_item_id, Utc::now());
    event.title = Some(title.to_string());
    event.intent = Some("Implement Workspace WorkItem lifecycle history".to_string());
    event.summary =
        Some("Workspace WorkItem history should be joined instead of duplicated.".to_string());
    event.status_category = Some(match kind {
        WorkEventKind::Done => WorkspaceStatusCategory::Done,
        _ => WorkspaceStatusCategory::Active,
    });
    event.agent_session_id = Some(session_id.to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    record_workspace_work_event(repo_path, event).expect("record workspace work item");
}

fn seed_issue_linkage(repo_path: &Path, branch: &str, issue_number: u64) {
    let repo_hash = compute_repo_hash("https://github.com/example/gwt-test.git");
    let store_path = repo_path
        .parent()
        .expect("repo parent")
        .join(".gwt/cache/issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    std::fs::create_dir_all(store_path.parent().expect("store parent")).expect("create store dir");
    std::fs::write(
        store_path,
        serde_json::to_vec_pretty(&json!({
            "branches": {
                branch: issue_number,
            }
        }))
        .expect("serialize linkage store"),
    )
    .expect("write linkage store");
}

#[test]
fn allows_read_only_tools_without_owner() {
    let event = event("Read", json!({ "file_path": "src/lib.rs" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "read-only tools must stay allowed");
}

#[test]
fn blocks_worktree_internal_edit_without_owner() {
    let wt = root();
    let event = event(
        "Edit",
        json!({ "file_path": format!("{}/src/lib.rs", wt.display()), "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("worktree-internal implementation edit must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_worktree_internal_edit_with_relative_path() {
    let event = event(
        "Edit",
        json!({ "file_path": "src/lib.rs", "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("relative implementation edit must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_edit_outside_worktree_without_owner() {
    let event = event(
        "Edit",
        json!({ "file_path": "/outside/project/src/lib.rs", "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("owner guard should block mutating edit without owner");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_docs_edit_outside_worktree_without_owner() {
    let event = event(
        "Edit",
        json!({ "file_path": "/outside/project/README.md", "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("owner guard should block outside-worktree docs edit");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn allows_docs_edits_without_owner_as_chore_exemption() {
    let event = event(
        "Edit",
        json!({ "file_path": "README.md", "old_string": "old", "new_string": "new" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "docs-only changes should stay allowed");
}

#[test]
fn allows_docs_only_apply_patch_without_owner_as_chore_exemption() {
    let event = event(
        "apply_patch",
        json!({
            "patch": "*** Begin Patch\n*** Update File: README.md\n@@\n-old\n+new\n*** End Patch\n"
        }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "docs-only apply_patch changes should stay allowed"
    );
}

#[test]
fn blocks_source_apply_patch_without_owner() {
    let event = event(
        "apply_patch",
        json!({
            "patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch\n"
        }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("source apply_patch without owner must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn allows_docs_only_apply_patch_for_spec_owner_before_plan_refresh() {
    let event = event(
        "apply_patch",
        json!({
            "patch": "*** Begin Patch\n*** Update File: docs/hooks.md\n@@\n-old\n+new\n*** End Patch\n"
        }),
    );
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, false, false),
    );
    assert!(
        decision.is_none(),
        "docs-only patch should not require spec plan/tasks"
    );
}

#[test]
fn allows_mutation_for_plain_issue_owner() {
    let event = event(
        "Write",
        json!({ "file_path": "src/lib.rs", "content": "fn x() {}\n" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::plain_issue(1942));
    assert!(
        decision.is_none(),
        "plain issue flow must not require spec plan/tasks"
    );
}

#[test]
fn allows_git_push_even_for_spec_without_plan() {
    let event = event("Bash", json!({ "command": "git push" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, false, true),
    );
    assert!(
        decision.is_none(),
        "git push is transport and must not be gated by plan/tasks"
    );
}

#[test]
fn allows_git_push_even_for_spec_without_tasks() {
    let event = event("Bash", json!({ "command": "git push" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, true, false),
    );
    assert!(
        decision.is_none(),
        "git push is transport and must not be gated by plan/tasks"
    );
}

#[test]
fn allows_spec_owner_when_plan_and_tasks_exist() {
    let event = event("Bash", json!({ "command": "git push origin main" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, true, true),
    );
    assert!(
        decision.is_none(),
        "ready spec owner should allow external ops"
    );
}

#[test]
fn allows_verification_bash_even_without_owner() {
    let event = event("Bash", json!({ "command": "cargo test -p gwt" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "verification commands should not be blocked by the workflow gate"
    );
}

#[test]
fn allows_worktree_touch_bash_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": format!("touch {}/src/lib.rs", root().display()) }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "worktree-local file ops should bypass the owner gate"
    );
}

#[test]
fn allows_worktree_rm_bash_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": format!("rm -f {}/.gwt/memory/constitution.md", root().display()) }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "worktree-local file ops should bypass the owner gate"
    );
}

#[test]
fn allows_cargo_fmt_without_owner() {
    let event = event("Bash", json!({ "command": "cargo fmt" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "cargo fmt is worktree-internal and must be allowed"
    );
}

#[test]
fn blocks_git_commit_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "git add . && git commit -m 'chore: release'" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("git commit without owner must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn allows_git_push_without_owner() {
    let event = event("Bash", json!({ "command": "git push origin main" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "git push is a transport operation and must not be gated by owner"
    );
}

#[test]
fn allows_harmless_echo_without_owner() {
    let event = event("Bash", json!({ "command": "echo test" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "harmless shell output should not be treated as implementation work"
    );
}

#[test]
fn allows_json_envelope_discovery_and_linking_without_owner() {
    for (operation, params) in [
        ("issue.view", json!({ "number": 3253 })),
        ("issue.comments", json!({ "number": 3253 })),
        ("issue.linked_prs", json!({ "number": 3253 })),
        ("issue.spec.read", json!({ "number": 1935 })),
        (
            "issue.spec.section",
            json!({ "number": 1935, "section": "spec" }),
        ),
        ("issue.spec.list", json!({ "state": "open" })),
        (
            "issue.create",
            json!({ "title": "bug", "body": "body", "labels": ["bug"] }),
        ),
        (
            "issue.spec.create",
            json!({ "title": "SPEC: test", "body": "body" }),
        ),
        (
            "issue.spec.edit",
            json!({ "number": 1935, "section": "plan", "body": "plan" }),
        ),
        ("pr.current", json!({})),
        ("pr.view", json!({ "number": 1 })),
        ("pr.checks", json!({ "number": 1 })),
        ("search", json!({ "query": "workflow policy owner" })),
        (
            "workspace.update",
            json!({ "current_focus": "checking owner" }),
        ),
        (
            "board.post",
            json!({ "kind": "status", "body": "checking owner" }),
        ),
        ("pane.list", json!({})),
        ("pane.read", json!({ "id": "pane-1" })),
    ] {
        let event = event(
            "Bash",
            json!({ "command": json_envelope_command(operation, params) }),
        );
        let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
        assert!(
            decision.is_none(),
            "{operation} JSON envelope should not be gated by owner"
        );
    }
}

#[test]
fn blocks_chained_json_envelope_plus_mutation_without_owner() {
    let command = format!(
        "{}\n&& git add . && git commit -m 'fix: hidden mutation'",
        json_envelope_command("issue.view", json!({ "number": 3253 }))
    );
    let event = event("Bash", json!({ "command": command }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("chained implementation mutation must still be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_json_envelope_redirect_without_owner() {
    let command = format!(
        "{} > output.json",
        json_envelope_command("issue.view", json!({ "number": 3253 }))
    );
    let event = event("Bash", json!({ "command": command }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("redirected JSON envelope output mutates the worktree");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn allows_git_push_with_session_bypass() {
    let event = event("Bash", json!({ "command": "git push origin main" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::with_bypass(gwt_agent::types::WorkflowBypass::Release),
    );
    assert!(decision.is_none(), "session bypass must allow git push");
}

#[test]
fn allows_git_push_with_chore_bypass() {
    let event = event("Bash", json!({ "command": "git push" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::with_bypass(gwt_agent::types::WorkflowBypass::Chore),
    );
    assert!(decision.is_none(), "chore bypass must allow git push");
}

#[test]
fn blocks_sed_in_place_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "sed -i 's/old/new/' Cargo.toml" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("sed -i without owner must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_shell_redirect_without_owner() {
    let event = event("Bash", json!({ "command": "echo '1.0.0' > version.txt" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("shell redirect without owner must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn allows_git_push_in_chained_command() {
    let event = event(
        "Bash",
        json!({ "command": "cargo fmt && git push origin main" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "chained command with git push must not be gated by owner"
    );
}

#[test]
fn worktree_external_file_op_is_blocked_before_owner_gate() {
    let event = event(
        "Bash",
        json!({ "command": format!("rm -rf {}", outside_root().display()) }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("out-of-worktree file ops must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("outside worktree"));
}

#[test]
fn reuses_legacy_bash_policy_rules_before_spec_gate() {
    let event = event("Bash", json!({ "command": "gh issue view 1935" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, true, true),
    )
    .expect("issue cli must still be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("GitHub workflow CLI"));
}

#[test]
fn evaluate_resolves_spec_owner_from_session_cache() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        seed_issue_cache(&repo_path, 1935, vec!["gwt-spec"], "", "- [ ] T-001");
        let session_id = save_session(&repo_path, "feature/workflow", Some(1935));
        seed_workspace_agent_title(&repo_path, &session_id);
        std::env::set_var(GWT_SESSION_ID_ENV, session_id);

        let event = event("Bash", json!({ "command": "git push" }));
        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");
        assert!(
            matches!(decision, HookOutput::Silent),
            "git push is transport and must not be gated by plan/tasks"
        );
    });
}

#[test]
fn evaluate_falls_back_to_issue_linkage_store_for_plain_issue_owner() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        seed_issue_cache(&repo_path, 1942, vec!["bug"], "n/a", "n/a");
        let session_id = save_session(&repo_path, "feature/workflow", None);
        seed_workspace_agent_title(&repo_path, &session_id);
        seed_issue_linkage(&repo_path, "feature/workflow", 1942);
        std::env::set_var(GWT_SESSION_ID_ENV, session_id);

        let event = event(
            "Write",
            json!({ "file_path": "src/lib.rs", "content": "fn x() {}\n" }),
        );
        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");
        assert!(
            matches!(decision, HookOutput::Silent),
            "plain issue owner from linkage store should allow implementation"
        );
    });
}

#[test]
fn similar_active_workspace_does_not_hard_block_mutation() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        seed_workspace_agents(
            &repo_path,
            &session_id,
            "Workspace semantic coordination gate",
            "session-other",
            "Workspace semantic coordination duplicate guard",
        );

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "active Workspace similarity is coordination context; duplicate prevention belongs to explicit workspace affiliation"
        );
    });
}

#[test]
fn allows_mutation_after_split_claim_targets_matching_workspace_agent() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        seed_workspace_agents(
            &repo_path,
            &session_id,
            "Workspace semantic coordination gate",
            "session-other",
            "Workspace semantic coordination duplicate guard",
        );

        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Claim,
            "Split accepted for same Workspace work.\n\nBoundary: current session owns workflow-policy tests and policy gate only.",
            None,
            None,
            vec!["workspace-semantic-coordination".to_string()],
            vec!["2359".to_string()],
        )
        .with_origin_session_id(session_id.clone())
        .with_mention(BoardMention::new(
            BoardMentionTargetKind::Session,
            "session-other",
        ));
        post_entry(&repo_path, entry).expect("post split claim");

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Boundary-targeted split claim should allow disjoint implementation"
        );
    });
}

#[test]
fn active_board_claim_does_not_hard_block_mutation() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        projection.agents.push(workspace_agent(
            &session_id,
            "Implement Workspace semantic coordination gate",
            "Workspace semantic coordination gate",
        ));
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Other Codex",
            BoardEntryKind::Claim,
            "Implement Workspace semantic coordination duplicate guard for active agents.",
            None,
            None,
            vec!["workspace-semantic-coordination".to_string()],
            vec!["2359".to_string()],
        )
        .with_origin_session_id("session-other");
        post_entry(&repo_path, entry).expect("post active claim");

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "active Board claims should coordinate duplicate risk without blocking unrelated tool execution"
        );
    });
}

#[test]
fn unassigned_agent_does_not_inherit_projection_title_for_duplicate_gate() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        projection.title = "Workspace affiliation fix".to_string();
        projection.summary = Some("Stale project-level workspace title".to_string());
        projection.status_category = WorkspaceStatusCategory::Active;
        projection
            .agents
            .push(unassigned_workspace_agent(&session_id));
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Other Codex",
            BoardEntryKind::Claim,
            "Workspace affiliation fix is in progress on another branch.",
            None,
            None,
            vec!["workspace-materialization".to_string()],
            vec!["2359".to_string()],
        )
        .with_origin_session_id("session-other");
        post_entry(&repo_path, entry).expect("post stale active claim");

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Unassigned Agents must not inherit stale projection-level title as duplicate-gate intent"
        );
    });
}

#[test]
fn does_not_block_when_active_board_claim_is_audienced_to_other_workspace() {
    // SPEC-2359 FR-099 / SC-031: a claim audienced only to a different
    // Workspace must not gate the current Agent. With Codex's
    // affiliation field landed, the current Agent is assigned to
    // `workspace-existing` (per workspace_agent helper); the claim
    // audienced to `ws-other-only` does not intersect, so the gate
    // must stay silent.
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        projection.agents.push(workspace_agent(
            &session_id,
            "Implement Workspace audience scoped gate",
            "Workspace audience scoped gate",
        ));
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Other Codex",
            BoardEntryKind::Claim,
            "Implement Workspace audience scoped gate for active agents.",
            None,
            None,
            vec!["workspace-audience".to_string()],
            vec!["2359".to_string()],
        )
        .with_origin_session_id("session-other")
        .with_audience(vec!["ws-other-only".to_string()]);
        post_entry(&repo_path, entry).expect("post audienced claim");

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        match decision {
            HookOutput::PreToolUsePermission { detail, .. } => {
                panic!(
                    "audience-only claim must not block the current Agent when audience does not intersect: {detail}"
                );
            }
            HookOutput::Silent => {}
            other => panic!("expected silent allow, got {other:?}"),
        }
    });
}

#[test]
fn unassigned_agent_without_title_summary_is_not_title_blocked() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        projection
            .agents
            .push(unassigned_workspace_agent(&session_id));
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/lib.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Unassigned Agents must not be blocked as missing title-summary"
        );
    });
}

#[test]
fn actionable_unassigned_agent_can_mutate_without_forced_workspace_affiliation() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        let mut agent = unassigned_workspace_agent(&session_id);
        agent.title_summary = Some("Workspace materialization".to_string());
        agent.current_focus = Some("Ensure actionable intent enters a Workspace".to_string());
        projection.agents.push(agent);
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/cli/workspace.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Unassigned is a valid coordination state; affiliation is explicit and optional"
        );
    });
}

#[test]
fn actionable_unassigned_agent_can_run_workspace_ensure_command() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        let mut agent = unassigned_workspace_agent(&session_id);
        agent.title_summary = Some("Workspace materialization".to_string());
        agent.current_focus = Some("Ensure actionable intent enters a Workspace".to_string());
        projection.agents.push(agent);
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let event = event(
            "Bash",
            json!({
                "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"workspace.ensure\",\"params\":{\"agent_session\":\"$GWT_SESSION_ID\",\"purpose\":\"Workspace materialization\",\"current_focus\":\"Ensure actionable intent enters a Workspace\",\"spec\":2359}}\nJSON"
            }),
        );

        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Workspace ensure must remain allowed so the Agent can repair affiliation"
        );
    });
}

#[test]
fn assigned_agent_without_title_summary_remains_title_blocked() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/assigned", None);
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        let mut agent = workspace_agent(&session_id, "Implement assigned work", "");
        agent.title_summary = None;
        projection.agents.push(agent);
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/lib.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::PreToolUsePermission { .. }),
            "Assigned Agents still need a title-summary before implementation"
        );
    });
}

#[test]
fn incomplete_work_item_history_does_not_hard_block_mutation() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        seed_workspace_current_agent(
            &repo_path,
            &session_id,
            "Workspace WorkItem history",
            "Implement Workspace WorkItem lifecycle history",
        );
        seed_workspace_work_item(
            &repo_path,
            "workitem-existing",
            WorkEventKind::Start,
            "Workspace WorkItem history duplicate prevention",
            "session-other",
        );

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt-core/src/workspace_projection.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "incomplete Workspace history is context; explicit workspace join/create owns duplicate prevention"
        );
    });
}

#[test]
fn completed_work_item_history_does_not_block_new_related_work() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        seed_workspace_current_agent(
            &repo_path,
            &session_id,
            "Workspace WorkItem history",
            "Implement Workspace WorkItem lifecycle history follow-up",
        );
        seed_workspace_work_item(
            &repo_path,
            "workitem-completed",
            WorkEventKind::Done,
            "Workspace WorkItem history",
            "session-other",
        );

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt-core/src/workspace_projection.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "completed WorkItem history must be context only"
        );
    });
}

// ---- #3267: owner guard read-only misclassification fixes ----

#[test]
fn allows_gh_release_view_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "gh release view v9.65.0 --repo akiojin/gwt --json isDraft,assets" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "gh release view is read-only");
}

#[test]
fn allows_gh_release_list_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "gh release list --repo akiojin/gwt --limit 1" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "gh release list is read-only");
}

#[test]
fn allows_gh_run_list_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "gh run list --workflow release.yml --branch main --limit 5 --repo akiojin/gwt" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "gh run list is read-only");
}

#[test]
fn blocks_gh_run_rerun_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "gh run rerun 123456 --failed --repo akiojin/gwt" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("gh run rerun mutates CI state and needs an owner or bypass");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_gh_release_create_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "gh release create v9.99.0 --repo akiojin/gwt" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("gh release create mutates release state");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn allows_stderr_dev_null_redirect_in_read_only_command() {
    for command in [
        "ls /tmp/does-not-exist 2>/dev/null",
        "grep -rn pattern src 2> /dev/null",
        "git log --oneline -3 2>/dev/null",
    ] {
        let event = event("Bash", json!({ "command": command }));
        let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
        assert!(
            decision.is_none(),
            "stderr-to-devnull must stay read-only: {command}"
        );
    }
}

#[test]
fn allows_stderr_merge_redirect_in_read_only_command() {
    let event = event(
        "Bash",
        json!({ "command": "git log --oneline -3 2>&1 | head -3" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "2>&1 does not write the worktree");
}

#[test]
fn blocks_stdout_file_redirect_even_with_stderr_merge() {
    let event = event(
        "Bash",
        json!({ "command": "git log --oneline > log.txt 2>&1" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("stdout file redirect still mutates the worktree");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn allows_git_ls_remote_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "git ls-remote --tags origin v9.65.0" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "git ls-remote is read-only transport");
}

#[test]
fn allows_git_rev_list_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "git rev-list v9.64.2..HEAD --count" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "git rev-list is read-only");
}

#[test]
fn allows_git_tag_queries_without_owner() {
    for command in [
        "git tag",
        "git tag --list 'v[0-9]*' --sort=-version:refname",
        "git tag -l 'v9.*'",
        "git tag --contains 7723d167d",
        "git tag --points-at HEAD",
    ] {
        let event = event("Bash", json!({ "command": command }));
        let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
        assert!(
            decision.is_none(),
            "git tag query must stay read-only: {command}"
        );
    }
}

#[test]
fn blocks_git_tag_mutations_without_owner() {
    for command in [
        "git tag v9.99.0",
        "git tag -d v9.99.0",
        "git tag -a v9.99.0 -m msg",
        "git tag -f v9.99.0 HEAD",
        "git tag --delete v9.99.0",
    ] {
        let event = event("Bash", json!({ "command": command }));
        let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
            .unwrap_or_else(|| panic!("git tag mutation must be blocked: {command}"));
        assert!(decision
            .permission_decision_reason()
            .contains("Owner Issue/SPEC"));
    }
}

#[test]
fn allows_gwt_bin_variable_json_envelope() {
    for (variable, operation) in [
        ("\"$GWT_BIN\"", "issue.view"),
        ("$GWT_BIN", "pr.view"),
        ("\"${GWT_BIN}\"", "issue.comment"),
    ] {
        let body = json!({
            "schema_version": 1,
            "operation": operation,
            "params": { "number": 3267 },
        });
        let command = format!("{variable} <<'JSON'\n{body}\nJSON");
        let event = event("Bash", json!({ "command": command }));
        let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
        assert!(
            decision.is_none(),
            "GWT_BIN-style standalone envelope must match literal gwtd: {command}"
        );
    }
}

#[test]
fn allows_sort_and_text_utils_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "grep -o 'x' file.txt | sort -u | uniq -c | cut -d: -f1 | tr -d ' '" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "text utils are read-only");
}

#[test]
fn allows_plan_file_write_under_home_claude_plans_without_owner() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("temp home");
    let _home_env = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
    let plan_path = home.path().join(".claude/plans/3267-release.md");
    let event = event(
        "Write",
        json!({ "file_path": plan_path.to_string_lossy(), "content": "# Plan" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "plan-mode plan files under ~/.claude/plans are documentation"
    );
}

#[test]
fn blocks_write_outside_worktree_non_plan_paths_without_owner() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("temp home");
    let _home_env = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
    let outside = home.path().join("elsewhere/notes.rs");
    let event = event(
        "Write",
        json!({ "file_path": outside.to_string_lossy(), "content": "fn x() {}" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("non-plan writes outside the worktree still need an owner");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

use super::*;
use gwt::cli::execution_state::{self as execution, ExecutionOwnerKey, ExecutionOwnerKind};

const CHILD_PHASE: &str = "GWT_RESTART_REGRESSION_PHASE";
const CHILD_HOME: &str = "GWT_RESTART_REGRESSION_HOME";
const OWNERS: [(u64, bool); 2] = [(2359, true), (2360, false)];

#[test]
fn host_restart_fresh_linked_owner_launch_supports_first_operations() {
    if let Some(phase) = std::env::var_os(CHILD_PHASE) {
        let home = PathBuf::from(std::env::var_os(CHILD_HOME).expect("child fixture HOME"));
        let _env_guard = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _home = ScopedGwtHome::set(&home);
        run_host_phase(&home, phase == "seed");
        return;
    }
    let home = tempdir().unwrap();
    for phase in ["seed", "restart"] {
        let mut child = gwt_core::process::hidden_command(std::env::current_exe().unwrap());
        child.args([
            "--exact",
            "app_runtime::tests::execution_restart_tests::host_restart_fresh_linked_owner_launch_supports_first_operations",
            "--nocapture",
        ]);
        // A fresh Host must not inherit authority from the agent running this test.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GWT_") {
                child.env_remove(key);
            }
        }
        let output = child
            .env(CHILD_PHASE, phase)
            .env(CHILD_HOME, home.path())
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .output()
            .expect("run isolated Host process");
        assert!(
            output.status.success(),
            "{phase} Host failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn run_host_phase(home: &Path, seed: bool) {
    let tabs = OWNERS
        .iter()
        .map(|(number, _)| {
            let repo = home.join(format!("repo-{number}"));
            if seed {
                fs::create_dir_all(&repo).unwrap();
                init_repo_with_initial_commit(&repo);
                run_git(&repo, &["branch", "-m", &format!("work/issue-{number}")]);
            }
            sample_project_tab_with_window_at(
                &format!("tab-{number}"),
                "agent-1",
                repo,
                WindowPreset::Agent,
                WindowProcessStatus::Running,
            )
        })
        .collect();
    let mut runtime = sample_runtime(&home.join("runtime"), tabs, Some("tab-2359"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    runtime.rebuild_window_lookup();
    let (spawner, _) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let tokio = TokioRuntime::new().unwrap();
    let (proxy, events) = AppEventProxy::stub();
    runtime.proxy = proxy.clone();
    let mut server = crate::embedded_server::EmbeddedServer::start(
        &tokio,
        proxy,
        crate::embedded_server::ClientHub::default(),
        Arc::clone(&runtime.pty_writers),
        AttachmentUploadStore::in_system_temp(),
    )
    .unwrap();
    let issuer = server.agent_capability_issuer();
    runtime.agent_capability_issuer = Some(issuer.clone());
    let prior_path = home.join("first-host.json");
    if seed {
        fs::write(
            &prior_path,
            serde_json::to_vec(&serde_json::json!({
                "pid": std::process::id(), "bridge": server.url()
            }))
            .unwrap(),
        )
        .unwrap();
    } else {
        let first: serde_json::Value =
            serde_json::from_slice(&fs::read(&prior_path).unwrap()).unwrap();
        assert_ne!(first["pid"].as_u64(), Some(u64::from(std::process::id())));
    }
    for (number, blocked_history) in OWNERS {
        let owner = ExecutionOwnerKey { kind: ExecutionOwnerKind::Issue, number };
        let repo = home.join(format!("repo-{number}"));
        if seed && blocked_history {
            execution::materialize_at_launch(&repo, owner.kind, number, "prior-blocked", "$gwt-execute", false).unwrap();
            execution::settle(&repo, "prior-blocked", execution::ExecutionSettlement::Blocked {
                reason: "previous verification was blocked".into(), missing_verification: None,
            }).unwrap();
        }
        let ledger_before = execution::load_generation_ledger(&repo, owner).unwrap();
        if !seed {
            let ledger = ledger_before.as_ref().expect("first Host persisted ledger");
            assert_eq!(ledger.current_effective_status(), Some(execution::ExecutionControlStatus::Active));
            assert_eq!(ledger.generations.len(), if blocked_history { 2 } else { 1 });
            let old = gwt_agent::Session::load(&runtime.sessions_dir.join(format!("before-restart-{number}.toml"))).unwrap();
            assert_eq!(old.status, gwt_agent::AgentStatus::Running);
        }
        let mut session = gwt_agent::Session::new(&repo, format!("work/issue-{number}"), gwt_agent::AgentId::Codex);
        session.id = format!("{}-restart-{number}", if seed { "before" } else { "after" });
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = Some(number);
        session.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, &session.id);
        gwt::session_launch::persist_finalized_launch_session(&runtime.sessions_dir, &runtime_path, &mut session, None).unwrap();
        let (env, pending) = super::super::launch::install_linked_owner_launch_for_restart_test(
            &issuer, &runtime.sessions_dir, &mut session, &repo, owner,
        );
        if seed {
            if let Some(pending) = pending {
                execution::activate_successor(&repo, owner, &pending.request).unwrap();
                clear_durable_launch_recovery(&runtime.sessions_dir, &session.id).unwrap();
            }
            let identity = gwt_agent::SessionExecutionIdentity::from_session(&session).unwrap().unwrap();
            let started = gwt::process::host_process_start_time(std::process::id()).unwrap();
            assert!(gwt_agent::persist_session_running_state_if_execution_identity_matches(
                &runtime.sessions_dir, &identity, number, started, std::process::id(), started,
            ).unwrap());
            continue;
        }
        let pending = pending.expect("restarted Host prepares a fresh successor");
        let binding = pending.binding.clone();
        let window = combined_window_id(&format!("tab-{number}"), "agent-1");
        let mut active = sample_active_agent_session(&format!("tab-{number}"), &window);
        active.session_id = session.id.clone();
        active.worktree_path = repo.clone();
        active.agent_project_root = repo.display().to_string();
        active.branch_name = session.branch.clone();
        runtime.active_agent_sessions.insert(window.clone(), active);
        runtime.agent_capability_tokens.insert(window.clone(), env[gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV].clone());
        runtime.pending_fresh_execution_launches.insert(window, pending);
        authenticated_readiness(&mut runtime, &events, &env, &session.id);
        assert!(issuer.active_token_is_current(&env[gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV], &binding));
        run_first_operations(&repo, &session, &runtime_path, &env, number);
    }
    server.shutdown();
}

fn authenticated_readiness(
    runtime: &mut AppRuntime,
    events: &Arc<Mutex<Vec<UserEvent>>>,
    env: &HashMap<String, String>,
    session_id: &str,
) {
    let request = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30)).build().unwrap()
        .post(&env[gwt_agent::GWT_HOOK_FORWARD_URL_ENV])
        .bearer_auth(&env[gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV])
        .json(&serde_json::json!({
            "kind": "runtime_state", "source_event": "SessionStart",
            "gwt_session_id": session_id,
            "continuation_readiness_nonce": env[gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV],
            "occurred_at": Utc::now().to_rfc3339()
        })).build().unwrap();
    let (response_tx, response_rx) = mpsc::channel();
    let sender = thread::spawn(move || {
        response_tx.send(reqwest::blocking::Client::new().execute(request).unwrap()).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let event = loop {
        let event = {
            let mut events = events.lock().unwrap();
            events.iter().position(|event| matches!(event, UserEvent::FreshExecutionReadyResend { .. } | UserEvent::RuntimeHook(_)))
                .map(|index| events.remove(index))
        };
        if let Some(event) = event { break event; }
        assert!(Instant::now() < deadline, "Host did not dispatch authenticated readiness");
        thread::sleep(Duration::from_millis(100));
    };
    assert!(
        matches!(response_rx.recv_timeout(Duration::from_secs(1)), Err(mpsc::RecvTimeoutError::Timeout)),
        "SessionStart acknowledged before the queued durable activation committed"
    );
    match event {
        UserEvent::FreshExecutionReadyResend { grant, request, reply } => {
            runtime.resend_fresh_execution_ready(&grant, &request, reply);
            commit_pending_fresh_execution(runtime);
        }
        UserEvent::RuntimeHook(event) => {
            runtime.handle_runtime_hook_event(event);
            commit_pending_fresh_execution(runtime);
        }
        _ => unreachable!(),
    }
    let response = response_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    sender.join().unwrap();
}

fn run_first_operations(repo: &Path, session: &gwt_agent::Session, runtime_path: &Path, env: &HashMap<String, String>, number: u64) {
    let _session = gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &session.id);
    let _runtime = gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, runtime_path);
    let _url = gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &env[gwt_agent::GWT_HOOK_FORWARD_URL_ENV]);
    let _token = gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, &env[gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV]);
    for (operation, params) in [
        ("workspace.ensure", serde_json::json!({"agent_session":session.id,"purpose":"Implement restarted owner","current_focus":"first operation after readiness","issue":number})),
        ("workspace.update", serde_json::json!({"agent_session":session.id,"purpose":"Implement restarted owner","current_focus":"verify restarted owner"})),
        ("verify.plan", serde_json::json!({"commands":["git status --short"]})),
        ("build.start", serde_json::json!({"spec":number})),
    ] {
        let mut cli = gwt::cli::TestEnv::new(repo.to_path_buf());
        cli.stdin = serde_json::json!({"schema_version":1,"operation":operation,"params":params}).to_string();
        let exit = gwt::cli::dispatch(&mut cli, &["gwtd".into()]);
        assert_eq!(exit, 0, "{operation} owner {number}: {}\n{}", String::from_utf8_lossy(&cli.stdout), String::from_utf8_lossy(&cli.stderr));
        let result: serde_json::Value = serde_json::from_slice(&cli.stdout).unwrap();
        assert_eq!(result["ok"], true, "{operation}: {result}");
    }
}

use super::*;

/// AC-6: one restarted Host must finalize all fresh launches behind the same
/// startup intake lock, including owners with persisted Blocked history.
#[test]
fn restarted_host_finalizes_six_fresh_sessions_after_shared_work_lock_contention() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _process_home = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
    let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/develop", "HEAD"],
    );
    gwt::agent_names::mutate_project_roster(
        &repo,
        &gwt_core::paths::gwt_sessions_dir(),
        gwt::agent_names::AgentRosterAction::Resize { size: 6 },
        None,
    )
    .unwrap();
    configure_owner_fixture_tools(&repo, &["fixture"]);
    // The fresh actor has no inherited GUI runtime or capability registry.
    let executor = TokioRuntime::new().unwrap();
    let owner = Arc::new(executor.block_on(production_owner_fixture(&repo)));
    let mut launches = Vec::new();
    for index in 0..6 {
        let number = 4940 + index;
        let branch = format!("work/issue-{number}");
        let worktree = temp.path().join(format!("worktree-{index}"));
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                &branch,
                worktree.to_str().unwrap(),
                "HEAD",
            ],
        );
        let operation_id = format!("restart-six-{index}");
        let (authority, choice) = owner_fresh_launch_seed(
            &worktree,
            &operation_id,
            gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number,
            index % 2 != 0,
        );
        let predecessor_binding = choice.expected_binding.clone();
        let fixture_executor = TokioRuntime::new().unwrap();
        let (fixture_owner, captured, pending) = fixture_executor.block_on(async {
            let owner = owner.clone();
            let checkpoint = owner.pause_next_agent_launch().await.unwrap();
            let mut command = owner_fresh_agent_command(&owner, &worktree, choice, None);
            let gwt::project_runtime::events::OwnerCommand::LaunchAgent { config, .. } =
                &mut command
            else {
                unreachable!("fresh successor launch")
            };
            // These six real children must remain live through all Work transactions;
            // the common fixture's timed provider can exit before the final readback.
            let provider = pane_control_tests::control_custom_agent();
            config.command = provider.command;
            config.args = provider.default_args;
            let client = owner.client();
            let launch = tokio::spawn(async move { client.command(command).await });
            // test-hygiene: allow-wall-clock-deadline Bounds the actual prepared successor checkpoint; no elapsed time establishes readiness.
            let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
                .await
                .expect("actual fresh preparation checkpoint");
            let pending = match captured.pending_fresh_execution.clone() {
                Some(pending) => pending,
                None => {
                    checkpoint.release();
                    let result = launch.await.expect("actual owner launch result");
                    if let Ok(owner) = Arc::try_unwrap(owner) {
                        owner.shutdown().await;
                    }
                    panic!(
                        "actual owner did not prepare the requested fresh successor: {result:?}"
                    );
                }
            };
            checkpoint.release();
            launch.await.unwrap().expect("physical fresh launch ACK");
            (owner, captured, pending)
        });
        let fixture = OwnerFreshLaunchFixture {
            executor: fixture_executor,
            owner: fixture_owner,
            captured,
            pending,
            repo: worktree,
            authority,
            predecessor_binding,
        };
        let predecessor_path = gwt_core::paths::gwt_sessions_dir()
            .join(format!("predecessor-restart-six-{index}.toml"));
        let predecessor_bytes = fs::read(&predecessor_path).unwrap();
        let generation =
            gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
                .unwrap()
                .unwrap()
                .generations[0]
                .clone();
        launches.push((fixture, predecessor_path, predecessor_bytes, generation));
    }
    let works_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    fs::create_dir_all(works_path.parent().unwrap()).unwrap();
    let lock_path = works_path.with_extension("lock");
    let holder = gwt_core::operation_deadline::NamedFileLock::acquire_quiet(
        &lock_path,
        "startup Work intake",
    )
    .unwrap();
    let (contended, contention) = mpsc::channel();
    let mut observers = Vec::new();
    let mut requests = Vec::new();
    for (fixture, ..) in &launches {
        assert_eq!(
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.repo),
            works_path,
            "all six linked worktrees share the canonical works.lock"
        );
        let candidate = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir()
                .join(format!("{}.toml", fixture.pending.binding.session_id)),
        )
        .unwrap();
        assert_eq!(
            gwt_agent::SessionExecutionIdentity::from_session(&candidate)
                .unwrap()
                .unwrap(),
            fixture.pending.session_identity
        );
        let probe = executor
            .block_on(owner.probe(&fixture.captured.id, fixture.captured.incarnation))
            .unwrap();
        let lock_path = lock_path.clone();
        let contended = contended.clone();
        observers.push(gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
            probe.grant.as_ref().unwrap(),
            gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::BeforeFreshWorkCommit,
            move || {
                let lock = OpenOptions::new().read(true).write(true).open(lock_path).unwrap();
                let error = lock.try_lock_exclusive().expect_err("startup intake owns works.lock");
                assert!(gwt_core::operation_deadline::is_lock_contended(&error), "{error}");
                contended.send(()).unwrap();
            }));
        let request = gwt::AgentExecutionContinuationRequest {
            schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
            operation_id: uuid::Uuid::new_v4().to_string(),
            readiness_nonce: Some(fixture.pending.readiness_nonce.clone()),
        };
        requests.push(executor.spawn(async move { probe.execution_continue(&request).await }));
    }
    drop(contended);
    for _ in 0..6 {
        // test-hygiene: allow-wall-clock-deadline hang guard for six actual authenticated owner workers reaching the held Work lock
        contention
            .recv_timeout(Duration::from_secs(30)) // test-hygiene: allow-wall-clock-deadline hang guard for real authenticated workers reaching the held canonical Work lock
            .expect("all six canonical Work transactions");
    }
    assert!(
        requests.iter().all(|request| !request.is_finished()),
        "no finalizer may complete while startup intake owns works.lock"
    );
    drop(holder);
    for request in requests {
        let (status, body) = executor.block_on(async {
            // test-hygiene: allow-wall-clock-deadline hang guard for the real HTTP readiness ACK after releasing works.lock
            tokio::time::timeout(Duration::from_secs(30), request)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        });
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(body["validated"], true);
    }
    let mut physical = Vec::new();
    for (fixture, predecessor_path, predecessor_bytes, generation) in &launches {
        let probe = executor
            .block_on(owner.probe(&fixture.captured.id, fixture.captured.incarnation))
            .unwrap();
        assert!(
            probe.pty.try_wait().unwrap().is_none(),
            "the exact successor must remain physically live through readiness"
        );
        physical.push((probe.id.clone(), probe.pty.clone()));
        assert!(probe.issuer.grant_is_current(probe.grant.as_ref().unwrap()));
        assert!(probe.has_capability_bearer(fixture.captured.grant.as_ref().unwrap()));
        assert!(probe.readiness_watch.is_none());
        assert!(!probe.readiness_inflight);
        let ledger =
            gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
                .unwrap()
                .unwrap();
        assert_eq!(ledger.generations.len(), 2);
        assert_eq!(&ledger.generations[0], generation);
        assert_eq!(fs::read(predecessor_path).unwrap(), *predecessor_bytes);
        let (status, body) = executor
            .block_on(probe.workspace_update(&gwt::AgentWorkspaceUpdateRequest {
                schema_version: gwt::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                claimed_session_id: fixture.pending.binding.session_id.clone(),
                observation: gwt::observe_agent_runtime(&fixture.repo).unwrap(),
                intent: gwt::AgentWorkspaceUpdateIntent {
                    current_focus: Some("six-session restart acceptance".into()),
                    ..Default::default()
                },
            }))
            .unwrap();
        assert!(status.is_success(), "workspace.update: {status}: {body}");
        probe.with_agent_environment(|| {
            let mut env = gwt::cli::TestEnv::new(fixture.repo.clone());
            let code = gwt::cli::run(
                &mut env,
                gwt::cli::CliCommand::Build(gwt::cli::SkillStateAction::Start {
                    spec: fixture.authority.number,
                }),
            )
            .unwrap();
            assert_eq!(
                code,
                0,
                "build.start: {}",
                String::from_utf8_lossy(&env.stdout)
            );
            assert!(
                gwt_core::skill_state::load(&fixture.repo, "build-spec")
                    .unwrap()
                    .unwrap()
                    .active
            );
            let code = gwt::cli::run(
                &mut env,
                gwt::cli::CliCommand::Verify(gwt::cli::verification_record::VerifyCommand::Plan {
                    commands: vec!["git status --porcelain".into()],
                    derive: false,
                }),
            )
            .unwrap();
            assert_eq!(
                code,
                0,
                "verify.plan: {}",
                String::from_utf8_lossy(&env.stdout)
            );
            assert_eq!(
                gwt::cli::verification_record::load_plan(&fixture.repo)
                    .unwrap()
                    .unwrap()
                    .session_id,
                fixture.pending.binding.session_id
            );
        });
    }
    drop(observers);
    for (fixture, ..) in launches {
        fixture.shutdown();
    }
    executor.block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("all fixture handles drained")
            .shutdown(),
    );
    assert_eq!(physical.len(), 6);
    for (id, pty) in physical {
        assert!(
            pty.tree_exit_confirmed(),
            "owner shutdown must reap the exact successor {id}"
        );
    }
}
#[test]
fn fresh_execution_session_start_queues_io_and_deduplicates_readiness() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _process_home = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
    let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-worker-readiness",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    let authority_runtime = Arc::new(TokioRuntime::new().unwrap());
    let waiting_runtime = authority_runtime.clone();
    let (entered, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let _observer = gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
        probe.grant.clone().as_ref().unwrap(),
        gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::BeforeFreshWorkCommit,
        move || {
            let grant = probe
                .current_capability_grant()
                .expect("same promoted bearer");
            assert!(grant.principal().authorizes_producing_mutation());
            assert_eq!(
                probe.issuer.durable_authority(&grant),
                crate::embedded_server::AgentDurableAuthority::PendingActivation
            );
            let mut waiter = waiting_runtime.spawn(
                gwt::agent_capability::durable_agent_execution_authority_with_lease_async(
                    grant.principal().clone(),
                ),
            );
            waiting_runtime.block_on(async {
                // test-hygiene: allow-wall-clock-deadline the actual activation fence stays held throughout this observation
                assert!(
                    tokio::time::timeout(Duration::from_millis(100), &mut waiter) // test-hygiene: allow-wall-clock-deadline hang guard while the actual activation fence is held at the pre-Work checkpoint
                        .await
                        .is_err(),
                    "the first pane mutation must wait while activation is fenced"
                );
            });
            entered.send(waiter).unwrap();
            released.recv().expect("readiness observer release");
        },
    );
    // These are actual transport events. The actor owns one in-flight job,
    // while the observed worker is held before the canonical Work transaction.
    fixture.session_start(Some(fixture.pending.readiness_nonce.clone()));
    // test-hygiene: allow-wall-clock-deadline hang guard for the exact owner worker pre-Work checkpoint
    let waiter = observed
        .recv_timeout(Duration::from_secs(30)) // test-hygiene: allow-wall-clock-deadline hang guard while the actual activation fence is held at the pre-Work checkpoint
        .expect("real readiness worker");
    fixture.session_start(Some(fixture.pending.readiness_nonce.clone()));
    let in_flight = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(
        in_flight.readiness_inflight,
        "duplicate readiness retains the same actor job"
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.predecessor_binding.clone())
    );
    assert!(load_tracked_work_events(&fixture.repo).is_empty());
    release.send(()).unwrap();
    commit_owner_fresh_readiness(&fixture);
    assert_eq!(
        authority_runtime.block_on(waiter).unwrap(),
        crate::embedded_server::AgentDurableAuthority::Current,
        "the same initial pane request must proceed after exact activation"
    );
    let promoted = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(!promoted.readiness_inflight);
    assert!(promoted.readiness_watch.is_none());
    let ledger =
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
            .unwrap()
            .unwrap();
    assert_eq!(ledger.generations.len(), 2);
    let work_events = load_tracked_work_events(&fixture.repo);
    assert_eq!(
        work_events
            .iter()
            .filter(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Start)
            .count(),
        1
    );
    fixture.session_start(Some(fixture.pending.readiness_nonce.clone()));
    fixture
        .executor
        .block_on(
            fixture
                .owner
                .command(gwt::project_runtime::events::OwnerCommand::Health),
        )
        .unwrap();
    let repeated = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(!repeated.readiness_inflight);
    assert_eq!(load_tracked_work_events(&fixture.repo), work_events);
    fixture.shutdown();
}
#[test]
fn fresh_execution_finalization_rejects_replaced_window_pending_and_token() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    use gwt::project_runtime::{events::OwnerCommand, launch_payload::ManualSuccessorChoice};
    for replacement in ["window", "pending", "token"] {
        let _case_home = ScopedGwtHome::set(temp.path().join(format!("home-{replacement}")));
        let fixture = owner_fresh_launch_fixture(
            temp.path(),
            replacement,
            gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            false,
            None,
        );
        let old = fixture
            .executor
            .block_on(
                fixture
                    .owner
                    .probe(&fixture.captured.id, fixture.captured.incarnation),
            )
            .unwrap();
        fixture
            .executor
            .block_on(fixture.owner.command(OwnerCommand::Close {
                id: old.id.clone(),
                incarnation: old.incarnation,
            }))
            .unwrap();
        assert_owner_fresh_launch_rolled_back(&fixture);
        let source = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir().join(format!("predecessor-{replacement}.toml")),
        )
        .unwrap();
        let source_identity = gwt_agent::SessionExecutionIdentity::from_session(&source)
            .unwrap()
            .unwrap();
        let replacement_operation = format!("replacement-{replacement}");
        let mut command = owner_fresh_agent_command(
            &fixture.owner,
            &fixture.repo,
            ManualSuccessorChoice {
                operation_id: replacement_operation.clone(),
                expected_binding: fixture.predecessor_binding.clone(),
                expected_predecessor: Some(Box::new(source_identity)),
                expected_runtime: None,
                predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor::Blocked,
            },
            None,
        );
        let OwnerCommand::LaunchAgent { id, .. } = &mut command else {
            unreachable!()
        };
        *id = old.id.clone();
        let receipt = fixture
            .executor
            .block_on(fixture.owner.command(command))
            .unwrap();
        let incarnation = receipt["incarnation"].as_u64().unwrap();
        let current = fixture
            .executor
            .block_on(fixture.owner.probe(&old.id, incarnation))
            .unwrap();
        let replacement_session = current.session.as_ref().unwrap().id.clone();
        assert_ne!(replacement_session, fixture.pending.binding.session_id);
        let current_grant = current.grant.as_ref().unwrap().clone();
        let work_before = load_tracked_work_events(&fixture.repo);
        match replacement {
            "window" => {
                assert!(
                    fixture
                        .executor
                        .block_on(fixture.owner.command(OwnerCommand::Close {
                            id: old.id.clone(),
                            incarnation: old.incarnation,
                        }))
                        .is_err(),
                    "old incarnation cannot retire its real replacement"
                );
            }
            "pending" => {
                assert!(
                    fixture
                        .executor
                        .block_on(fixture.owner.check_readiness_deadline(
                            &old.id,
                            incarnation,
                            ContinueWorkReadinessWatch::new(fixture.pending.operation_id.clone())
                        ))
                        .is_err(),
                    "old operation cannot replace the current pending coordinator"
                );
            }
            "token" => {
                let (status, _) = fixture
                    .executor
                    .block_on(
                        old.execution_continue(&gwt::AgentExecutionContinuationRequest {
                            schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                            operation_id: uuid::Uuid::new_v4().to_string(),
                            readiness_nonce: Some(fixture.pending.readiness_nonce.clone()),
                        }),
                    )
                    .unwrap();
                assert!(
                    !status.is_success(),
                    "revoked real old bearer cannot finalize its replacement"
                );
            }
            _ => unreachable!(),
        }
        let retained = fixture
            .executor
            .block_on(fixture.owner.probe(&old.id, incarnation))
            .unwrap();
        assert_eq!(
            retained.readiness_watch.as_ref().unwrap().operation_id,
            replacement_operation
        );
        assert_eq!(retained.session.as_ref().unwrap().id, replacement_session);
        assert!(retained.has_capability_bearer(&current_grant));
        assert!(retained.issuer.grant_is_current(&current_grant));
        assert!(retained.pty.try_wait().unwrap().is_none());
        assert_eq!(load_tracked_work_events(&fixture.repo), work_before);
        fixture.shutdown();
    }
}

#[test]
fn fresh_execution_queued_readiness_cannot_revive_an_exact_rollback() {
    assert_owner_queued_readiness_close("fresh-worker-rollback", false, false);
}

#[test]
fn fresh_execution_queued_session_start_rejects_foreign_identity() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-early-readiness",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let original = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    let mut early = runtime_hook_state_for_event("Working", "SessionStart", "foreign-session");
    early.agent_session_id = Some(fixture.pending.binding.session_id.clone());
    early.continuation_readiness_nonce = Some(fixture.pending.readiness_nonce.clone());
    early.project_root = Some(fixture.repo.display().to_string());
    early.branch = Some("work/issue-2359".to_string());
    fixture
        .owner
        .transport_event(gwt::project_transport::TransportEvent::RuntimeHook(
            early.clone(),
        ));
    let foreign = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(
        !foreign.readiness_inflight,
        "foreign identity cannot authenticate readiness"
    );
    assert_eq!(foreign.readiness_watch, original.readiness_watch);
    assert!(foreign
        .issuer
        .grant_is_current(foreign.grant.as_ref().unwrap()));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.predecessor_binding.clone())
    );
    early.gwt_session_id = Some(fixture.pending.binding.session_id.clone());
    fixture
        .owner
        .transport_event(gwt::project_transport::TransportEvent::RuntimeHook(early));
    commit_owner_fresh_readiness(&fixture);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.pending.binding.identity.clone())
    );
    fixture.shutdown();
}

#[test]
fn fresh_execution_queued_readiness_rolls_back_after_pane_or_project_close() {
    for (operation, project_close, close_worker_first) in [
        ("pane-readiness-first", false, false),
        ("pane-close-first", false, true),
        ("project-close-first", true, true),
    ] {
        assert_owner_queued_readiness_close(operation, project_close, close_worker_first);
    }
}

fn assert_owner_queued_readiness_close(
    operation: &str,
    project_close: bool,
    close_worker_first: bool,
) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        operation,
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        fixture.repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let context = runtime.test_context();
    let snapshot = fixture
        .executor
        .block_on(
            fixture
                .owner
                .command(gwt::project_runtime::events::OwnerCommand::Snapshot {
                    id: fixture.captured.id.clone(),
                    incarnation: fixture.captured.incarnation,
                }),
        )
        .unwrap();
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    runtime.install_remote_owner_projection(
        &context,
        "tab-1::agent-1",
        fixture.owner.client(),
        serde_json::from_value(snapshot).unwrap(),
        probe.session.clone(),
    );
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let (entered, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let observer = (!close_worker_first).then(|| {
        gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(probe.grant.as_ref().unwrap(),
            gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::BeforeFreshWorkCommit,
            move || { entered.send(()).unwrap(); released.recv().unwrap(); })
    });
    if !close_worker_first {
        fixture.session_start(Some(fixture.pending.readiness_nonce.clone()));
        // test-hygiene: allow-wall-clock-deadline hang guard for actual readiness entering its pre-Work checkpoint
        observed.recv_timeout(Duration::from_secs(30)).unwrap();
    }
    if project_close {
        runtime.close_project_tab_events("tab-1");
    } else {
        assert!(runtime.close_window_outcome("tab-1::agent-1").closed);
    }
    assert!(
        !runtime.runtimes.contains_key("tab-1::agent-1"),
        "accepted close removes the GUI read model"
    );
    let jobs = std::mem::take(&mut *tasks.lock().unwrap());
    assert!(
        !jobs.is_empty(),
        "physical close must execute off the GUI thread"
    );
    let close_worker = thread::spawn(move || {
        for job in jobs {
            job();
        }
    });
    fixture.executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline hang guard for actual close RPC admission before releasing readiness
        tokio::time::timeout(Duration::from_secs(30), async {
            while fixture.captured.current.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    if !close_worker_first {
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &fixture.repo,
                fixture.authority,
                &fixture.pending.operation_id
            )
            .unwrap()
            .unwrap()
            .status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
        );
        assert!(
            probe.pty.try_wait().unwrap().is_none(),
            "close ACK cannot precede its queued physical retirement"
        );
        release.send(()).unwrap();
    } else {
        fixture.session_start(Some(fixture.pending.readiness_nonce.clone()));
    }
    close_worker.join().unwrap();
    apply_recorded_window_close_finalized(&mut runtime, &recorded);
    assert_owner_fresh_launch_rolled_back(&fixture);
    fixture.session_start(Some(fixture.pending.readiness_nonce.clone()));
    fixture
        .executor
        .block_on(
            fixture
                .owner
                .command(gwt::project_runtime::events::OwnerCommand::Health),
        )
        .unwrap();
    assert_owner_fresh_launch_rolled_back(&fixture);
    assert!(load_tracked_work_events(&fixture.repo).is_empty());
    drop(observer);
    fixture.shutdown();
}

#[test]
fn fresh_execution_completed_predecessor_survives_terminal_grace_and_activates_once() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "completed-requeue",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        true,
        None,
    );
    let now = Instant::now();
    fixture
        .executor
        .block_on(fixture.owner.tick_runtime_policy(now))
        .unwrap();
    fixture
        .executor
        .block_on(
            fixture
                .owner
                .tick_runtime_policy(now + Duration::from_secs(61)),
        )
        .unwrap();
    let pending = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .expect("Completed predecessor cannot retire its Prepared successor");
    assert!(pending.current.load(Ordering::Acquire));
    assert!(pending.pty.try_wait().unwrap().is_none());
    assert!(pending.readiness_watch.is_some());
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.predecessor_binding.clone())
    );
    commit_owner_fresh_readiness(&fixture);
    let active = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(
        active.readiness_watch.is_none(),
        "activated runtime leaves the Prepared terminal exemption"
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.pending.binding.identity.clone())
    );
    let ledger =
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
            .unwrap()
            .unwrap();
    assert_eq!(ledger.generations.len(), 2);
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Active)
    );
    fixture.shutdown();
}

#[test]
fn durable_launch_recovery_receipt_is_monotonic_and_replays_base_write_without_downgrade() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "fresh-receipt-monotonic");
    let expected = durable_launch_recovery_session_identity(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    )
    .expect("read bound receipt")
    .expect("bound identity");

    persist_durable_launch_recovery(
        &fixture.runtime.sessions_dir,
        DurableLaunchRecoveryKind::FreshSuccessor {
            operation_id: fixture.operation_id.clone(),
        },
        &fixture.candidate_session_id,
        &fixture.repo,
        &fixture.repo,
        fixture.owner,
        None,
        None,
    )
    .expect("a retry must retain the stronger exact recovery identity");
    assert_eq!(
        durable_launch_recovery_session_identity(
            &fixture.runtime.sessions_dir,
            &fixture.candidate_session_id,
        )
        .expect("read retained bound receipt"),
        Some(expected)
    );
}

#[test]
fn prepared_manual_successor_loser_is_rejected_before_pane_creation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "manual-pre-pane-claim");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&fixture.repo)
        .branch("work/issue-2359")
        .linked_issue_number(fixture.owner.number)
        .extra_arg("$gwt-execute #2359")
        .build();
    config.execution_intent =
        gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(fixture.binding.clone());
    let winner = gwt::project_runtime::continuation::claim_prepared_manual_successor_launch(
        &fixture.runtime.sessions_dir,
        &fixture.repo,
        &config,
    )
    .expect("claim Prepared candidate")
    .expect("manual successor claim");
    let claim_error = gwt::project_runtime::continuation::claim_prepared_manual_successor_launch(
        &fixture.runtime.sessions_dir,
        &fixture.repo,
        &config,
    )
    .expect_err("the canonical claim must reject another materializer");
    assert!(
        claim_error.contains("already being materialized"),
        "{claim_error}"
    );
    let tab = sample_project_tab("tab-1", "Repo", fixture.repo.clone(), ProjectKind::Git, &[]);
    let runtime_root = fixture
        .runtime
        .sessions_dir
        .parent()
        .expect("runtime root")
        .to_path_buf();
    let mut losing_runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));

    let error = losing_runtime
        .spawn_agent_window("tab-1", config, None)
        .expect_err("the concurrent Prepared materializer must lose before pane creation");

    assert!(
        error.contains("Prepared launch authority must remain in the project owner"),
        "{error}"
    );
    assert!(losing_runtime.window_lookup.is_empty());
    assert!(losing_runtime
        .tab("tab-1")
        .expect("loser tab")
        .workspace
        .persisted()
        .windows
        .is_empty());
    assert!(
        gwt::cli::execution_state::finish_active_session_launch_handshake(
            &fixture.runtime.sessions_dir,
            &winner,
        )
        .expect("release winner claim")
    );
}

#[test]
fn fresh_execution_session_start_routes_monitor_ack_to_feedback_owner_project() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_monitor_fresh_fixture(temp.path(), "monitor-owner-routing", false, false);
    let foreign = temp.path().join("active-gui-project");
    fs::create_dir_all(&foreign).unwrap();
    init_repo_with_initial_commit(&foreign);
    let foreign_path = gwt::issue_monitor_prefs_path_for_repo_path(&foreign);
    gwt::save_issue_monitor_prefs(
        &foreign_path,
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 900,
                window_id: "foreign::sentinel".into(),
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let foreign_before = fs::read(&foreign_path).unwrap();
    let tabs = vec![sample_project_tab(
        "active",
        "Foreign",
        foreign,
        ProjectKind::Git,
        &[],
    )];
    let _gui = sample_runtime(temp.path(), tabs, Some("active"));
    commit_owner_fresh_readiness(&fixture);
    wait_owner_monitor_fresh_ack(&fixture);
    assert_eq!(
        fs::read(&foreign_path).unwrap(),
        foreign_before,
        "active GUI project cannot redirect the fixed owner's Monitor ACK"
    );
    let prefs =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo))
            .unwrap();
    assert_eq!(
        prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: fixture.authority.number,
            window_id: fixture.captured.id.clone()
        }]
    );
    assert!(prefs.launching_issues.is_empty());
    fixture.shutdown();
}

/// SPEC #3200 FR-052: a fresh linked-owner launch is not producing until
/// SessionStart authenticates, so `spawn_agent_window_with_options` defers
/// the Issue Monitor completion. The SessionStart finalizer therefore owns the
/// durable delivery ACK — if it emitted the plain "launch succeeded" events the
/// delivery tuple would stay pending forever and the daemon would redeliver it.
#[test]
fn fresh_execution_session_start_acks_durable_issue_monitor_launch_delivery() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_monitor_fresh_fixture(temp.path(), "fresh-monitor-delivery", true, false);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo);
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .pending_launch_deliveries
            .len(),
        1
    );
    let scope = gwt_core::daemon::RuntimeScope::from_project_root(
        fixture
            .pending
            .launch_feedback_context
            .as_ref()
            .unwrap()
            .issue_monitor_project_root
            .as_ref()
            .unwrap(),
        gwt_core::daemon::RuntimeTarget::Host,
    )
    .unwrap();
    let endpoint = scope.endpoint_path(&gwt_core::paths::gwt_home());
    let endpoint_bytes = fs::read(&endpoint).unwrap();
    fs::write(&endpoint, b"{\"pid\":").unwrap();
    commit_owner_fresh_readiness(&fixture);
    wait_owner_monitor_fresh_ack(&fixture);
    fs::write(&endpoint, endpoint_bytes).unwrap();
    let projection = gwt_core::workspace_projection::load_workspace_projection(&fixture.repo)
        .unwrap()
        .unwrap();
    let agent = projection
        .latest_agent_for_session(&fixture.pending.binding.session_id)
        .unwrap();
    assert!(agent.is_assigned());
    let work_id = agent.workspace_id.as_deref().unwrap();
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .unwrap()
        .unwrap();
    assert!(work_items.work_items.iter().any(|item| item.id == work_id
        && item
            .agents
            .iter()
            .any(|agent| agent.session_id == fixture.pending.binding.session_id)));
    assert!(gwt::load_issue_monitor_prefs(&prefs_path).unwrap().pending_launch_deliveries.is_empty(), "authenticated readiness must durably consume its deferred exact delivery despite malformed endpoint fallback");
    fixture.shutdown();
}

fn wait_owner_monitor_fresh_ack(fixture: &OwnerFreshLaunchFixture) {
    // test-hygiene: allow-wall-clock-deadline Bounds an actual owner/daemon durable ACK; the persisted exact binding supplies the assertion.
    fixture.executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds actual owner/daemon delivery; the persisted exact binding is the success condition.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let prefs = gwt::load_issue_monitor_prefs(
                    &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo),
                )
                .unwrap();
                if prefs.pending_launch_deliveries.is_empty()
                    && prefs.launched_issues.iter().any(|row| {
                        row.issue_number == fixture.authority.number
                            && row.window_id == fixture.captured.id
                    })
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    });
}

/// Issue #5288 AC-2: a failed fresh Session must settle its exact Monitor
/// delivery before closing, so another scan cannot materialize it again.
#[test]
fn fresh_monitor_startup_failure_stops_delivery_once() {
    assert_fresh_monitor_startup_failures_stop(&["direct", "worker"]);
}

#[test]
fn issue_5415_fresh_monitor_startup_timeout_requeues_without_spending_an_attempt() {
    assert_fresh_monitor_startup_failures_stop(&["startup-timeout", "startup-timeout-replay"]);
}

#[test]
fn issue_5415_startup_timeout_preserves_submitted_and_replaced_sessions() {
    assert_fresh_monitor_startup_failures_stop(&["startup-submitted", "startup-replaced"]);
}

#[test]
fn fresh_monitor_readiness_failure_stops_delivery_once() {
    assert_fresh_monitor_startup_failures_stop(&["readiness"]);
}

#[test]
fn fresh_monitor_activation_failure_keeps_rejected_check_and_owners() {
    assert_fresh_monitor_startup_failures_stop(&["activation"]);
}

#[test]
fn fresh_monitor_failure_replay_closes_only_its_exact_candidate() {
    assert_fresh_monitor_startup_failures_stop(&[
        "replay",
        "readiness-replay",
        "prepty-replay",
        "replacement",
    ]);
}

fn assert_fresh_monitor_startup_failures_stop(failures: &[&str]) {
    use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // State outcomes use the existing SPEC #4740 worker budget seam.
    let _prefs_budget = ScopedEnvVar::set(
        "GWT_TEST_BUDGET_ISSUE_MONITOR_PREFS_MS",
        gwt_core::deadline_budget::HANG_GUARD
            .as_millis()
            .to_string(),
    );
    for &failure in failures {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let prepty = matches!(failure, "direct" | "worker" | "prepty-replay");
        let fixture = owner_monitor_fresh_fixture(
            temp.path(),
            &format!("fresh-monitor-{failure}"),
            true,
            prepty,
        );
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo);
        let id = fixture.captured.id.clone();
        let incarnation = fixture.captured.incarnation;
        if matches!(failure, "startup-submitted" | "startup-replaced") {
            let initial = fixture
                .executor
                .block_on(fixture.owner.probe(&id, incarnation))
                .unwrap();
            let due = initial.monitor_startup_at.unwrap();
            if failure == "startup-submitted" {
                commit_owner_fresh_readiness(&fixture);
                wait_owner_monitor_fresh_ack(&fixture);
                let mut hook = runtime_hook_coordination_event("foreign-session");
                hook.source_event = Some("UserPromptSubmit".into());
                hook.project_root = Some(fixture.repo.display().to_string());
                hook.branch = Some("work/issue-2359".into());
                fixture
                    .owner
                    .transport_event(gwt::project_transport::TransportEvent::RuntimeHook(
                        hook.clone(),
                    ));
                fixture
                    .executor
                    .block_on(fixture.owner.command(OwnerCommand::Health))
                    .unwrap();
                assert!(
                    fixture
                        .executor
                        .block_on(fixture.owner.probe(&id, incarnation))
                        .unwrap()
                        .monitor_startup_at
                        .is_some(),
                    "a foreign Session cannot prove prompt submission"
                );
                hook.gwt_session_id = Some(fixture.pending.binding.session_id.clone());
                fixture
                    .owner
                    .transport_event(gwt::project_transport::TransportEvent::RuntimeHook(hook));
                fixture
                    .executor
                    .block_on(fixture.owner.command(OwnerCommand::Health))
                    .unwrap();
                let current = fixture
                    .executor
                    .block_on(fixture.owner.probe(&id, incarnation))
                    .unwrap();
                assert!(current.monitor_startup_at.is_none());
                let before = fs::read(&prefs_path).unwrap();
                fixture
                    .executor
                    .block_on(fixture.owner.check_monitor_deadlines(due))
                    .unwrap();
                assert_eq!(fs::read(&prefs_path).unwrap(), before);
                assert!(current.pty.try_wait().unwrap().is_none());
            } else {
                fixture
                    .executor
                    .block_on(fixture.owner.command(OwnerCommand::Close {
                        id: id.clone(),
                        incarnation,
                    }))
                    .unwrap();
                let root = fixture
                    .pending
                    .launch_feedback_context
                    .as_ref()
                    .unwrap()
                    .issue_monitor_project_root
                    .as_ref()
                    .unwrap();
                let raw = id.split_once("::").unwrap().1;
                let successor = super::pane_control_tests::control_owner_launch(
                    &fixture.owner,
                    raw,
                    super::pane_control_tests::control_launch_config(root, 2360),
                );
                let before = fs::read(&prefs_path).unwrap();
                fixture
                    .executor
                    .block_on(fixture.owner.check_monitor_deadlines(due))
                    .unwrap();
                assert_eq!(fs::read(&prefs_path).unwrap(), before);
                let current = fixture
                    .executor
                    .block_on(fixture.owner.probe(&id, successor.incarnation))
                    .unwrap();
                assert!(Arc::ptr_eq(&current.pty, &successor.pty));
                assert_ne!(successor.incarnation, incarnation);
                assert!(successor.pty.try_wait().unwrap().is_none());
                super::pane_control_tests::control_close(&fixture.owner, &successor);
            }
            fixture.shutdown();
            continue;
        }
        let deferred = matches!(
            failure,
            "replay" | "readiness-replay" | "replacement" | "startup-timeout-replay"
        );
        // New/retried held-phase transactions use 250ms; started transactions
        // retain the budget captured before the holder was acquired.
        let held_prefs_budget =
            deferred.then(|| ScopedEnvVar::set("GWT_TEST_BUDGET_ISSUE_MONITOR_PREFS_MS", "250"));
        let holder = deferred.then(|| {
            gwt_core::operation_deadline::NamedFileLock::acquire_quiet(
                &prefs_path.with_extension("lock"),
                "Monitor failure commit fixture",
            )
            .unwrap()
        });
        let mut events = fixture.owner.subscribe();
        if !prepty {
            if failure.starts_with("startup-timeout") {
                let current = fixture
                    .executor
                    .block_on(fixture.owner.probe(&id, incarnation))
                    .unwrap();
                fixture
                    .executor
                    .block_on(
                        fixture
                            .owner
                            .check_monitor_deadlines(current.monitor_startup_at.unwrap()),
                    )
                    .unwrap();
            } else {
                if failure == "activation" {
                    let work_id = gwt_core::workspace_projection::canonical_work_id(
                        &fixture.repo,
                        Some("work/issue-2359"),
                        Some(&fixture.repo),
                    )
                    .unwrap();
                    let mut start = gwt_core::workspace_projection::WorkEvent::new(
                        gwt_core::workspace_projection::WorkEventKind::Start,
                        &work_id,
                        Utc::now(),
                    );
                    start.owner = Some("Issue #999".into());
                    start.execution_container = Some(
                        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                            branch: Some("work/issue-2359".into()),
                            worktree_path: Some(fixture.repo.clone()),
                            pr_number: None,
                            pr_url: None,
                            pr_state: None,
                        },
                    );
                    gwt_core::workspace_projection::record_workspace_work_event(
                        &fixture.repo,
                        start,
                    )
                    .unwrap();
                }
                fixture.session_start(Some(if failure == "activation" {
                    fixture.pending.readiness_nonce.clone()
                } else {
                    "wrong-nonce".into()
                }));
            }
            // test-hygiene: allow-wall-clock-deadline Bounds the exact real owner physical-close result; that receipt determines whether the Monitor commit remains pending.
            let close_error = fixture.executor.block_on(async {
                // test-hygiene: allow-wall-clock-deadline Bounds an actual owner physical close or canonical failure ACK; the receipt and durable source establish ordering.
                tokio::time::timeout(Duration::from_secs(30), async {
                    loop {
                        if let OwnerEvent::Closed {
                            id: closed,
                            incarnation: generation,
                            error,
                            ..
                        } = events.recv().await.unwrap()
                        {
                            if closed == id && generation == incarnation {
                                break error;
                            }
                        }
                    }
                })
                .await
                .unwrap()
            });
            if deferred {
                assert!(
                    close_error.is_some(),
                    "the held canonical prefs lock must prevent a successful close acknowledgement"
                );
                assert!(durable_launch_recovery_exists(
                    &gwt_core::paths::gwt_sessions_dir(),
                    &fixture.pending.binding.session_id
                ));
                let health = fixture
                    .executor
                    .block_on(fixture.owner.command(OwnerCommand::Health))
                    .unwrap();
                assert_eq!(health["reconciliation_required"], true);
                assert_eq!(
                    gwt::load_issue_monitor_prefs(&prefs_path)
                        .unwrap()
                        .pending_launch_deliveries
                        .len(),
                    1
                );
            } else {
                assert!(close_error.is_none(), "{failure}: {close_error:?}");
            }
        }
        drop(holder);
        drop(held_prefs_budget);
        if deferred {
            fixture
                .executor
                .block_on(fixture.owner.command(OwnerCommand::Close {
                    id: id.clone(),
                    incarnation,
                }))
                .unwrap();
        }
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert!(
            prefs.pending_launch_deliveries.is_empty(),
            "{failure}: exact delivery must settle before owned window removal"
        );
        assert!(!durable_launch_recovery_exists(
            &gwt_core::paths::gwt_sessions_dir(),
            &fixture.pending.binding.session_id
        ));
        assert!(!gwt_core::paths::gwt_sessions_dir()
            .join(format!("{}.toml", fixture.pending.binding.session_id))
            .exists());
        assert!(!gwt_agent::runtime_state_path(
            &gwt_core::paths::gwt_sessions_dir(),
            &fixture.pending.binding.session_id
        )
        .exists());
        assert!(!fixture
            .captured
            .issuer
            .grant_is_current(fixture.captured.grant.as_ref().unwrap()));
        if let Some(pty) = fixture.captured.pty.as_ref() {
            assert!(pty.tree_exit_confirmed());
        }
        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
                .unwrap(),
            Some(fixture.predecessor_binding.clone())
        );
        let attempt = gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.authority,
            &fixture.pending.operation_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            attempt.status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Aborted
        );
        let monitor = owner_monitor_read(&fixture.repo, 2359);
        assert_eq!(monitor.attempt_count(2359), 0);
        let reason = monitor
            .inbox_item(2359)
            .unwrap()
            .error_message
            .as_deref()
            .expect("durable actionable failure");
        if failure.starts_with("startup-timeout") {
            assert!(reason.starts_with("Issue Monitor startup timed out after "));
            assert!(gwt_agent::is_transient_launch_failure(reason));
            assert_eq!(
                monitor.inbox_item(2359).unwrap().state,
                gwt::MonitorInboxState::Queued
            );
        } else {
            assert_eq!(
                monitor.inbox_item(2359).unwrap().state,
                gwt::MonitorInboxState::AgentFailed
            );
        }
        let before = fs::read(&prefs_path).unwrap();
        assert!(
            fixture
                .executor
                .block_on(
                    fixture
                        .owner
                        .command(OwnerCommand::AcknowledgeMonitorLaunch {
                            id: id.clone(),
                            incarnation,
                            issue_number: 2359,
                            delivery_id: Some("launch:effect-2359".into())
                        })
                )
                .is_err(),
            "a stale failed candidate cannot ACK a consumed delivery"
        );
        assert_eq!(
            fs::read(&prefs_path).unwrap(),
            before,
            "failure replay cannot spend another attempt or erase its reason"
        );
        if failure == "replacement" {
            let root = fixture
                .pending
                .launch_feedback_context
                .as_ref()
                .unwrap()
                .issue_monitor_project_root
                .as_ref()
                .unwrap();
            let successor = super::pane_control_tests::control_owner_launch(
                &fixture.owner,
                id.split_once("::").unwrap().1,
                super::pane_control_tests::control_launch_config(root, 2360),
            );
            assert!(fixture
                .executor
                .block_on(fixture.owner.command(OwnerCommand::Close {
                    id: id.clone(),
                    incarnation
                }))
                .is_err());
            assert!(successor.pty.try_wait().unwrap().is_none());
            super::pane_control_tests::control_close(&fixture.owner, &successor);
        }
        fixture.shutdown();
    }
}

#[test]
fn fresh_execution_session_start_preserves_spec_owner_kind_in_work_projection() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-spec-owner",
        gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        false,
        None,
    );
    commit_owner_fresh_readiness(&fixture);
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .expect("SPEC WorkItems")
        .expect("Work");
    assert!(work_items
        .work_items
        .iter()
        .any(|item| item.owner.as_deref() == Some("SPEC-2359")
            && item
                .agents
                .iter()
                .any(|agent| agent.session_id == fixture.pending.binding.session_id)));
    fixture.shutdown();
}

// #3426: a stale Issue-kind resume context supplied to a SPEC-owner successor
// must not overwrite the canonical owner in the committed Work projection.
#[test]
fn fresh_execution_session_start_overrides_mis_kinded_resume_context_owner() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-mis-kinded-context",
        gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        false,
        Some(WorkspaceResumeContext {
            title: Some("Stale launch title".into()),
            owner: Some("Issue #2359".into()),
            summary: Some("Stale summary".into()),
            next_action: None,
        }),
    );
    assert_eq!(
        fixture
            .pending
            .resume_context
            .as_ref()
            .and_then(|context| context.owner.as_deref()),
        Some("Issue #2359"),
        "the immutable launch intent must reach the actual owner before canonical correction"
    );
    commit_owner_fresh_readiness(&fixture);
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .expect("WorkItems")
        .expect("Work");
    let work = work_items
        .work_items
        .iter()
        .find(|item| {
            item.agents
                .iter()
                .any(|agent| agent.session_id == fixture.pending.binding.session_id)
        })
        .expect("actual successor Work");
    assert_eq!(
        work.owner.as_deref(),
        Some("SPEC-2359"),
        "canonical owner overrides the stale presentation context"
    );
    assert_eq!(
        work.title, "Stale launch title",
        "non-owner context fields remain presentation-owned"
    );
    fixture.shutdown();
}

#[test]
fn fresh_execution_ready_timeout_aborts_candidate_and_preserves_blocked_predecessor() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-timeout-operation",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
    let mut events = fixture.owner.subscribe();
    let pty = fixture
        .captured
        .pty
        .as_ref()
        .expect("actual installed candidate");
    gwt::session_finalizer::kill_and_reap(pty, &fixture.captured.id).unwrap();
    assert!(matches!(
        continue_work_readiness_decision(
            &ContinueWorkReadinessWatch::new(fixture.pending.operation_id.clone()),
            ReadinessPaneEvidence::Dead,
            0
        ),
        ReadinessDeadlineDecision::Abort { .. }
    ));
    fixture.executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline hang guard for exact actual owner retirement after the captured process tree exits
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match events.recv().await.unwrap() {
                    OwnerEvent::Closed {
                        id,
                        incarnation,
                        error,
                        ..
                    } if id == fixture.captured.id
                        && incarnation == fixture.captured.incarnation =>
                    {
                        assert!(
                            error.is_none(),
                            "actual exact terminal retirement: {error:?}"
                        );
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(
            fixture
                .owner
                .check_readiness_deadline(
                    &fixture.captured.id,
                    fixture.captured.incarnation,
                    ContinueWorkReadinessWatch::new(fixture.pending.operation_id.clone())
                )
                .await
                .is_err(),
            "an old deadline cannot revive a candidate after exact physical cleanup"
        );
        fixture.owner.command(OwnerCommand::Health).await.unwrap();
    });
    assert_owner_fresh_launch_rolled_back(&fixture);
    fixture.shutdown();
}

#[test]
fn fresh_execution_session_replacement_before_work_commit_preserves_predecessor_authority() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-session-replacement-before-commit",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    let original_session = probe.session.clone().expect("actual installed source");
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let session_id = fixture.pending.binding.session_id.clone();
    let (replaced, observed) = mpsc::channel();
    let _observer = gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
        probe.grant.clone().as_ref().unwrap(),
        gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::BeforeFreshWorkCommit,
        move || {
            replace_fresh_candidate_session_incarnation(&sessions, &session_id);
            let grant = probe
                .current_capability_grant()
                .expect("original capability");
            assert_eq!(
                probe.issuer.durable_authority(&grant),
                crate::embedded_server::AgentDurableAuthority::Stale,
                "a replaced Session must not be mistaken for pending activation"
            );
            replaced.send(()).unwrap();
        },
    );
    let work_before = tracked_work_event_store_snapshot(&fixture.repo);
    let response = fixture.executor.block_on(async {
        let request_probe = fixture
            .owner
            .probe(&fixture.captured.id, fixture.captured.incarnation)
            .await
            .unwrap();
        request_probe
            .execution_continue(&gwt::AgentExecutionContinuationRequest {
                schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: uuid::Uuid::new_v4().to_string(),
                readiness_nonce: Some(fixture.pending.readiness_nonce.clone()),
            })
            .await
            .unwrap()
    });
    assert!(
        !response.0.is_success(),
        "a replaced Session cannot complete the fresh launch"
    );
    // test-hygiene: allow-wall-clock-deadline hang guard for the exact pre-Work Session replacement observer
    observed.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.predecessor_binding.clone())
    );
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.authority,
            &fixture.pending.operation_id
        )
        .unwrap()
        .unwrap()
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
    );
    assert_eq!(
        tracked_work_event_store_snapshot(&fixture.repo),
        work_before
    );
    // Restore only the fixture's deliberately replaced source before exact
    // physical teardown; the refusal above has already proved fail-closed CAS.
    original_session
        .save(&gwt_core::paths::gwt_sessions_dir())
        .unwrap();
    fixture.shutdown();
}

#[test]
fn issue_monitor_scan_reconciles_activated_launch_but_preserves_unready_candidate() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Both setup and ScanNow assert state outcomes under the existing worker seam.
    let _prefs_budget = ScopedEnvVar::set(
        "GWT_TEST_BUDGET_ISSUE_MONITOR_PREFS_MS",
        gwt_core::deadline_budget::HANG_GUARD
            .as_millis()
            .to_string(),
    );
    for activated in [false, true] {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let fixture = owner_monitor_fresh_fixture(
            temp.path(),
            if activated {
                "monitor-scan-activated"
            } else {
                "monitor-scan-unready"
            },
            true,
            false,
        );
        if activated {
            leave_owner_fresh_activated_before_projection_commit(&fixture);
        }
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo);
        assert_eq!(
            gwt::load_issue_monitor_prefs(&prefs_path)
                .unwrap()
                .pending_launch_deliveries
                .len(),
            1
        );
        gwt::daemon_publisher::publish_durable_issue_monitor_control(
            &fixture.repo,
            gwt::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({"scan_now":true}),
                std::process::id(),
            ),
        )
        .unwrap();
        fixture
            .executor
            .block_on(fixture.owner.wait_monitor_idle())
            .unwrap();
        if activated {
            wait_owner_monitor_fresh_ack(&fixture);
        }
        let probe = fixture
            .executor
            .block_on(
                fixture
                    .owner
                    .probe(&fixture.captured.id, fixture.captured.incarnation),
            )
            .unwrap();
        assert!(
            probe.pty.try_wait().unwrap().is_none(),
            "scan must preserve the exact physical candidate"
        );
        assert_eq!(
            probe.readiness_watch.is_some(),
            !activated,
            "only already Activated authority may repair its deferred readiness"
        );
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        if activated {
            assert!(prefs.pending_launch_deliveries.is_empty());
            assert_eq!(
                prefs.launched_issues,
                vec![gwt::IssueMonitorLaunchedIssue {
                    issue_number: fixture.authority.number,
                    window_id: fixture.captured.id.clone()
                }]
            );
            assert_eq!(
                gwt::cli::execution_state::current_execution_binding(
                    &fixture.repo,
                    fixture.authority
                )
                .unwrap(),
                Some(fixture.pending.binding.identity.clone())
            );
        } else {
            assert_eq!(prefs.pending_launch_deliveries.len(), 1);
            assert!(prefs.launched_issues.is_empty());
            assert_eq!(
                gwt::cli::execution_state::current_execution_binding(
                    &fixture.repo,
                    fixture.authority
                )
                .unwrap(),
                Some(fixture.predecessor_binding.clone())
            );
            assert!(durable_launch_recovery_exists(
                &gwt_core::paths::gwt_sessions_dir(),
                &fixture.pending.binding.session_id
            ));
        }
        fixture.shutdown();
    }
}

#[cfg(windows)]
#[test]
fn fresh_monitor_powershell_session_start_activates_prepared_successor_and_acknowledges() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture =
        owner_monitor_fresh_fixture(temp.path(), "monitor-powershell-ready", false, false);
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("gwtd.exe");
    assert!(
        binary.is_file(),
        "build the checkout gwtd binary before this integration test"
    );
    let spaced_bin = temp.path().join("hook bin");
    fs::create_dir_all(&spaced_bin).unwrap();
    let hook_bin = spaced_bin.join("gwtd.exe");
    fs::copy(binary, &hook_bin).unwrap();
    gwt_skills::generate_codex_hooks_for_mode(
        &fixture.repo,
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal,
    )
    .unwrap();
    let hooks: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.repo.join(".codex/hooks.json")).unwrap()).unwrap();
    let command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let output = probe.with_agent_environment(|| {
        let mut child = gwt_core::process::hidden_command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", command])
            .current_dir(&fixture.repo)
            .env(gwt_agent::GWT_BIN_PATH_ENV, &hook_bin)
            .env(
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV,
                &fixture.pending.readiness_nonce,
            )
            .env_remove("CODEX_THREAD_ID")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                serde_json::json!({
                    "session_id":"native-monitor-powershell", "cwd":fixture.repo,
                    "hook_event_name":"SessionStart", "source":"startup"
                })
                .to_string()
                .as_bytes(),
            )
            .unwrap();
        child.wait_with_output().unwrap()
    });
    assert!(
        output.status.success(),
        "generated startup hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait_owner_monitor_fresh_ack(&fixture);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.pending.binding.identity.clone())
    );
    let promoted = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(promoted.has_capability_bearer(fixture.captured.grant.as_ref().unwrap()));
    assert!(promoted
        .issuer
        .grant_is_current(promoted.grant.as_ref().unwrap()));
    let prefs =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo))
            .unwrap();
    assert!(prefs.launching_issues.is_empty());
    assert_eq!(
        prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: fixture.authority.number,
            window_id: fixture.captured.id.clone()
        }]
    );
    fixture.shutdown();
}

#[test]
fn fresh_execution_continue_resends_ready_and_commits_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-continue-ready",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    let (entered, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let _observer = gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
        probe.grant.as_ref().unwrap(),
        gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::AfterFreshActivationBeforePublication,
        move || { entered.send(()).unwrap(); released.recv().unwrap(); });
    let request = gwt::AgentExecutionContinuationRequest {
        schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
        operation_id: uuid::Uuid::new_v4().to_string(),
        readiness_nonce: Some(fixture.pending.readiness_nonce.clone()),
    };
    assert!(!format!("{request:?}").contains(request.readiness_nonce.as_deref().unwrap()));
    let first_request = request.clone();
    let first = fixture
        .executor
        .spawn(async move { probe.execution_continue(&first_request).await });
    // test-hygiene: allow-wall-clock-deadline hang guard for actual ledger activation before canonical Work publication
    observed
        .recv_timeout(Duration::from_secs(30)) // test-hygiene: allow-wall-clock-deadline hang guard for actual activation before canonical Work publication
        .expect("real activation checkpoint");
    assert!(
        !first.is_finished(),
        "Prepared request must await canonical publication"
    );
    let active = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(active.readiness_inflight);
    let active_request = gwt::AgentExecutionContinuationRequest {
        schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
        operation_id: uuid::Uuid::new_v4().to_string(),
        readiness_nonce: None,
    };
    let expected_active_request = active_request.operation_id.clone();
    let second = fixture
        .executor
        .spawn(async move { active.execution_continue(&active_request).await });
    fixture.executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline hang guard for the second actual HTTP request joining the captured readiness job
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let waiting = fixture
                    .owner
                    .probe(&fixture.captured.id, fixture.captured.incarnation)
                    .await
                    .unwrap();
                if waiting.readiness_waiters == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    assert!(
        !second.is_finished(),
        "Active resend must join the real in-flight finalization"
    );
    release.send(()).unwrap();
    let receipt: gwt::AgentExecutionContinuationReceipt = fixture.executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline hang guard for actual Prepared readiness HTTP ACK after Work publication
        let (status, body) = tokio::time::timeout(Duration::from_secs(30), first)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(status.is_success(), "{status}: {body}");
        serde_json::from_value(body).unwrap()
    });
    assert!(receipt.validated);
    assert_eq!(receipt.operation_id, request.operation_id);
    assert_eq!(receipt.execution_binding, fixture.pending.binding.identity);
    let active_receipt: gwt::AgentExecutionContinuationReceipt = fixture.executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline hang guard for the coalesced Active readiness HTTP ACK
        let (status, body) = tokio::time::timeout(Duration::from_secs(30), second)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(status.is_success(), "{status}: {body}");
        serde_json::from_value(body).unwrap()
    });
    assert_eq!(active_receipt.operation_id, expected_active_request);
    assert_eq!(
        active_receipt.execution_binding,
        fixture.pending.binding.identity
    );
    let works = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .unwrap()
        .unwrap();
    let work_id = gwt_core::workspace_projection::current_work_id(
        &works,
        &fixture.repo,
        Some("work/issue-2359"),
        Some(&fixture.repo),
    )
    .unwrap();
    assert_eq!(receipt.work_id.as_deref(), Some(work_id.as_str()));
    let active = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(active
        .issuer
        .grant_is_current(active.grant.as_ref().unwrap()));
    assert!(active.has_capability_bearer(fixture.captured.grant.as_ref().unwrap()));
    assert!(active.readiness_watch.is_none());
    let ledger =
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
            .unwrap()
            .unwrap();
    assert_eq!(ledger.generations.len(), 2);
    assert_eq!(
        ledger.generations[0].status,
        gwt::cli::execution_state::ExecutionControlStatus::Blocked
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.pending.binding.identity.clone())
    );
    let (status, body) = fixture
        .executor
        .block_on(active.workspace_update(&gwt::AgentWorkspaceUpdateRequest {
            schema_version: gwt::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
            claimed_session_id: fixture.pending.binding.session_id.clone(),
            observation: gwt::observe_agent_runtime(&fixture.repo).unwrap(),
            intent: gwt::AgentWorkspaceUpdateIntent {
                current_focus: Some("verify fresh authority".into()),
                ..Default::default()
            },
        }))
        .unwrap();
    assert!(
        status.is_success(),
        "workspace.update with promoted bearer: {status}: {body}"
    );
    let (status, replay) = fixture
        .executor
        .block_on(active.execution_continue(&request))
        .unwrap();
    assert!(
        status.is_success(),
        "ready response-loss replay: {status}: {replay}"
    );
    let replay: gwt::AgentExecutionContinuationReceipt = serde_json::from_value(replay).unwrap();
    assert_eq!(replay.generation_id, receipt.generation_id);
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
            .unwrap()
            .unwrap()
            .generations
            .len(),
        2
    );
    fixture.shutdown();
}

fn leave_owner_fresh_activated_before_projection_commit(fixture: &OwnerFreshLaunchFixture) {
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    probe.with_agent_environment(|| {
        let token = std::env::var(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV).unwrap();
        probe
            .issuer
            .promote_prepared(&token, &fixture.pending.binding)
            .unwrap();
    });
    let active = probe.active_session.as_ref().unwrap();
    let transaction = gwt_core::workspace_projection::transact_workspace_state_for_work_event_root_with_supersession_commit(
        &fixture.pending.project_root, &fixture.pending.worktree_path, &fixture.pending.operation_id,
        |projection, work_items, _| {
            let events = gwt::project_runtime::workspace::apply_workspace_launch_for_current_work(
                &fixture.pending.project_root, projection, work_items, active,
                gwt::project_runtime::workspace::WorkspaceLaunchTransition {
                    work_id: None, base_branch: fixture.pending.base_branch.as_deref(),
                    linked_issue_number: fixture.pending.linked_issue_number,
                    canonical_owner: Some(fixture.pending.owner), resume_context: fixture.pending.resume_context.as_ref(),
                    kind: WorkspaceLaunchProjectionKind::StartWork,
                    live_session_ids: &HashSet::from([fixture.pending.binding.session_id.clone()]), now: Utc::now(),
                }, true)?;
            Ok(((), events))
        },
        || {
            gwt::cli::execution_state::activate_successor(&fixture.repo, fixture.authority,
                &fixture.pending.request).map_err(gwt_core::error::GwtError::Io)?;
            Err(gwt_core::error::GwtError::Other("simulated response loss after ledger activation".into()))
        });
    assert!(transaction.is_err());
    let trusted = gwt::cli::trusted_store::trusted_dir_for_worktree(&fixture.repo).unwrap();
    for path in [
        trusted.join("execution-control.json"),
        trusted.join("execution-generation-pointer.json"),
        fixture
            .repo
            .join(gwt::cli::execution_state::EXECUTION_CONTROL_STATE_RELATIVE),
        fixture
            .repo
            .join(gwt::cli::execution_state::EXECUTION_GENERATION_POINTER_STATE_RELATIVE),
    ] {
        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
    assert!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .is_err()
    );
    assert!(durable_launch_recovery_exists(
        &gwt_core::paths::gwt_sessions_dir(),
        &fixture.pending.binding.session_id
    ));
}
#[test]
fn fresh_execution_continue_repairs_activated_response_loss_before_acknowledging() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-continue-response-loss",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    leave_owner_fresh_activated_before_projection_commit(&fixture);
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    let (status, body) = fixture
        .executor
        .block_on(
            probe.execution_continue(&gwt::AgentExecutionContinuationRequest {
                schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: uuid::Uuid::new_v4().to_string(),
                readiness_nonce: None,
            }),
        )
        .unwrap();
    assert!(
        status.is_success(),
        "actual Active readiness retry must repair before ACK: {status}: {body}"
    );
    assert_eq!(body["validated"], true);
    let repaired = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(repaired.readiness_watch.is_none());
    assert!(!repaired.readiness_inflight);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.pending.binding.identity.clone())
    );
    assert!(!durable_launch_recovery_exists(
        &gwt_core::paths::gwt_sessions_dir(),
        &fixture.pending.binding.session_id
    ));
    fixture.shutdown();
}

#[test]
fn fresh_execution_continue_refuses_wrong_nonce_without_mutation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-continue-wrong-nonce",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let before =
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
            .expect("Prepared ledger");
    let diagnosis = gwt::cli::execution_state::diagnose(
        &fixture.repo,
        Some(&fixture.pending.binding.session_id),
    );
    assert!(
        !diagnosis
            .available_recoveries
            .iter()
            .any(|operation| operation == "execution.adopt" || operation == "execution.continue"),
        "Prepared recovery requires Host readiness proof: {diagnosis:?}"
    );
    assert_eq!(
        diagnosis.recovery_hint.as_deref(),
        Some("prepared_launch_readiness_required")
    );
    let files = snapshot_optional_files(&[gwt_core::paths::gwt_sessions_dir()
        .join(format!("{}.toml", fixture.pending.binding.session_id))]);
    let work = tracked_workspace_work_store_snapshot(&fixture.repo);
    fixture.executor.block_on(async {
        use gwt::project_runtime::events::OwnerEvent;
        let mut events = fixture.owner.subscribe();
        let probe = fixture
            .owner
            .probe(&fixture.captured.id, fixture.captured.incarnation)
            .await
            .expect("actual Prepared runtime");
        let (status, _error) = probe
            .execution_continue(&gwt::AgentExecutionContinuationRequest {
                schema_version: 1,
                operation_id: "continue-ready-request".into(),
                readiness_nonce: Some("wrong-nonce".into()),
            })
            .await
            .expect("actual authenticated owner refusal");
        assert!(
            !status.is_success(),
            "wrong nonce cannot acknowledge readiness"
        );
        fixture
            .owner
            .command(gwt::project_runtime::events::OwnerCommand::Health)
            .await
            .expect("refusal actor barrier");
        assert!(
            !std::iter::from_fn(|| events.try_recv().ok()).any(
                |event| matches!(event, OwnerEvent::Status { id, .. } if id == fixture.captured.id)
            ),
            "a refused resend must not mutate or publish candidate status"
        );
        let current = fixture
            .owner
            .probe(&fixture.captured.id, fixture.captured.incarnation)
            .await
            .expect("same exact pending launch");
        assert!(current.issuer.grant_is_current(
            fixture
                .captured
                .grant
                .as_ref()
                .expect("actual Prepared grant")
        ));
    });
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
            .expect("unchanged ledger"),
        before
    );
    assert_optional_files_unchanged(&files);
    assert_tracked_workspace_work_store_unchanged(&fixture.repo, &work);
    fixture.shutdown();
}

#[test]
fn fresh_execution_wrong_session_start_nonce_aborts_candidate_and_preserves_blocked_predecessor() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(temp.path(), "fresh_execution_wrong_session_start_nonce_aborts_candidate_and_preserves_blocked_predecessor",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue, false, None);
    let detail = reject_owner_fresh_readiness(&fixture, Some("wrong-readiness".into()));
    assert!(
        detail.contains("readiness nonce did not match"),
        "the actual owner must publish the nonce refusal: {detail}"
    );
    assert!(
        load_tracked_work_events(&fixture.repo).is_empty(),
        "rejected readiness cannot create phantom Work"
    );
    assert_owner_fresh_launch_rolled_back(&fixture);
    fixture.shutdown();
}

#[test]
fn fresh_execution_missing_session_start_nonce_aborts_candidate_and_preserves_blocked_predecessor()
{
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(temp.path(), "fresh_execution_missing_session_start_nonce_aborts_candidate_and_preserves_blocked_predecessor",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue, false, None);
    let detail = reject_owner_fresh_readiness(&fixture, None);
    assert!(
        detail.contains("readiness nonce did not match"),
        "the actual owner must publish the nonce refusal: {detail}"
    );
    assert!(
        load_tracked_work_events(&fixture.repo).is_empty(),
        "rejected readiness cannot create phantom Work"
    );
    assert_owner_fresh_launch_rolled_back(&fixture);
    fixture.shutdown();
}

#[test]
fn fresh_execution_session_start_via_daemon_fanout_preserves_readiness_nonce() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-daemon-fanout",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let mut session_start = runtime_hook_state_for_event(
        "Working",
        "SessionStart",
        &fixture.pending.binding.session_id,
    );
    session_start.continuation_readiness_nonce = Some(fixture.pending.readiness_nonce.clone());
    session_start.project_root = Some(fixture.repo.display().to_string());
    session_start.branch = Some("work/issue-2359".into());
    // This codec origin is loop-prevention metadata; the actual Session,
    // readiness nonce, PTY and capability below all come from the real owner.
    let current_pid = std::process::id();
    let payload = gwt::runtime_daemon_events::runtime_hook_payload(
        &session_start,
        current_pid.wrapping_add(1),
    );
    let gwt::runtime_daemon_events::RuntimeDaemonEvent::Hook { event } =
        gwt::runtime_daemon_events::decode_runtime_daemon_event(
            gwt::runtime_daemon_events::RUNTIME_HOOK_CHANNEL,
            payload,
            current_pid,
        )
        .unwrap()
    else {
        panic!("expected runtime hook fanout");
    };
    assert_eq!(
        event.continuation_readiness_nonce.as_deref(),
        Some(fixture.pending.readiness_nonce.as_str())
    );
    fixture
        .owner
        .transport_event(gwt::project_transport::TransportEvent::RuntimeHook(event));
    fixture.executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline Observe completion of the preceding real fanout before authenticating a separate positive HTTP request; no scheduler delay or elapsed-time assertion.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let active = fixture
                    .owner
                    .probe(&fixture.captured.id, fixture.captured.incarnation)
                    .await
                    .expect("the actual fanout's exact runtime");
                if active.readiness_watch.is_none()
                    && !active.readiness_inflight
                    && active.pending_fresh_execution.is_none()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the fanout must complete actual readiness");
    });
    commit_owner_fresh_readiness(&fixture);
    let active = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(active.readiness_watch.is_none());
    assert_eq!(
        active.active_session.as_ref().unwrap().session_id,
        fixture.pending.binding.session_id
    );
    assert!(active.has_capability_bearer(fixture.captured.grant.as_ref().unwrap()));
    assert!(active
        .issuer
        .grant_is_current(active.grant.as_ref().unwrap()));
    fixture.shutdown();
}

#[test]
fn fresh_execution_session_start_requires_gwt_session_identity_before_nonce_validation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-foreign-gwt-session",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let before =
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
            .expect("Prepared ledger");
    fixture.executor.block_on(async {
        use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
        let mut events = fixture.owner.subscribe();
        let probe = fixture
            .owner
            .probe(&fixture.captured.id, fixture.captured.incarnation)
            .await
            .expect("actual Prepared runtime");
        let watch = probe
            .readiness_watch
            .clone()
            .expect("actual armed readiness deadline");
        let mut foreign =
            runtime_hook_state_for_event("Working", "SessionStart", "foreign-gwt-session");
        foreign.agent_session_id = Some(fixture.pending.binding.session_id.clone());
        foreign.continuation_readiness_nonce = Some(fixture.pending.readiness_nonce.clone());
        foreign.project_root = Some(fixture.repo.display().to_string());
        foreign.branch = Some("work/issue-2359".into());
        fixture
            .owner
            .transport_event(gwt::project_transport::TransportEvent::RuntimeHook(foreign));
        fixture
            .owner
            .command(OwnerCommand::Health)
            .await
            .expect("actual foreign hook barrier");
        let current = fixture
            .owner
            .probe(&fixture.captured.id, fixture.captured.incarnation)
            .await
            .expect("same exact pending owner pane");
        assert_eq!(current.readiness_watch.as_ref(), Some(&watch));
        assert!(
            current.issuer.grant_is_current(
                fixture
                    .captured
                    .grant
                    .as_ref()
                    .expect("actual Prepared grant")
            ),
            "foreign gwt identity cannot activate or abort the actual candidate"
        );
        assert!(
            !std::iter::from_fn(|| events.try_recv().ok()).any(
                |event| matches!(event, OwnerEvent::Status { id, .. } if id == fixture.captured.id)
            ),
            "foreign identity cannot emit candidate status"
        );
    });
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.authority)
            .expect("unchanged ledger"),
        before
    );
    fixture.shutdown();
}

#[test]
fn fresh_execution_worker_spawn_failure_aborts_its_reconstructed_candidate() {
    assert_owner_fresh_worker_completion("physical-failure");
}

#[test]
fn fresh_execution_worker_dropped_completion_aborts_exact_prepared_candidate() {
    assert_owner_fresh_worker_completion("dropped-mailbox");
}

#[test]
fn fresh_execution_worker_stale_completion_preserves_activated_generation() {
    assert_owner_fresh_worker_completion("activated-cancel");
}

#[test]
fn fresh_execution_spawn_failure_aborts_candidate_and_preserves_blocked_predecessor() {
    assert_owner_fresh_worker_completion("preparation-failure");
}

fn assert_owner_fresh_worker_completion(scenario: &str) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    run_git(&repo, &["branch", "-M", "work/issue-2359"]);
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/develop", "HEAD"],
    );
    let operation_id = format!("fresh-worker-{scenario}");
    let (authority, choice) = owner_fresh_launch_seed(
        &repo,
        &operation_id,
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        2359,
        false,
    );
    let predecessor = choice.expected_binding.clone();
    let predecessor_path =
        gwt_core::paths::gwt_sessions_dir().join(format!("predecessor-{operation_id}.toml"));
    let predecessor_bytes = fs::read(&predecessor_path).unwrap();
    let work_before = tracked_work_event_store_snapshot(&repo);
    let executor = TokioRuntime::new().unwrap();
    executor.block_on(async {
        use gwt::project_runtime::{events::OwnerCommand, test_support::OwnerLaunchStage};
        let owner = production_owner_fixture(&repo).await;
        let stage = if scenario == "physical-failure" || scenario == "preparation-failure" {
            OwnerLaunchStage::MaterializedBeforeInstall
        } else {
            OwnerLaunchStage::PreparedBeforeMailbox
        };
        let checkpoint = owner.pause_next_agent_launch_at(stage).await.unwrap();
        let mut command = owner_fresh_agent_command(&owner, &repo, choice, None);
        if scenario == "physical-failure" || scenario == "preparation-failure" {
            let OwnerCommand::LaunchAgent { config, .. } = &mut command else {
                unreachable!()
            };
            config.command = temp
                .path()
                .join("missing-candidate-program")
                .display()
                .to_string();
            config.args.clear();
        }
        let client = owner.client();
        let launch = tokio::spawn(async move { client.command(command).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual candidate preparation checkpoint
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        let pending = captured
            .pending_fresh_execution
            .as_ref()
            .expect("real prepared successor")
            .clone();
        let cancel = if scenario == "dropped-mailbox" {
            owner.disconnect_preparing_mailbox().await.unwrap();
            None
        } else if scenario == "activated-cancel" {
            gwt::cli::execution_state::activate_successor(&repo, authority, &pending.request)
                .unwrap();
            let client = owner.client();
            let id = captured.id.clone();
            let operation_id = operation_id.clone();
            let cancel = tokio::spawn(async move {
                client
                    .command(OwnerCommand::PendingCancel { id, operation_id })
                    .await
            });
            // test-hygiene: allow-wall-clock-deadline hang guard for real cancellation admission before releasing the worker
            tokio::time::timeout(Duration::from_secs(30), async {
                while captured.current.load(Ordering::Acquire) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            Some(cancel)
        } else {
            None
        };
        checkpoint.release();
        assert!(
            launch.await.unwrap().is_err(),
            "failed or cancelled candidate cannot install"
        );
        if let Some(cancel) = cancel {
            cancel.await.unwrap().unwrap();
        }
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let candidate_path = sessions.join(format!("{}.toml", pending.binding.session_id));
        if scenario == "dropped-mailbox" {
            // test-hygiene: allow-wall-clock-deadline hang guard for the actual SendError retirement finishing durable candidate rollback
            tokio::time::timeout(Duration::from_secs(30), async {
                while candidate_path.exists()
                    || captured
                        .grant
                        .as_ref()
                        .is_some_and(|grant| captured.issuer.grant_is_current(grant))
                    || captured
                        .pty
                        .as_ref()
                        .is_some_and(|pty| !pty.tree_exit_confirmed())
                    || gwt::persistence::load_workspace_state(
                        &gwt::persistence::workspace_state_path(&repo),
                    )
                    .map(|state| {
                        state.windows.iter().any(|row| {
                            row.session_id.as_deref() == Some(pending.binding.session_id.as_str())
                        })
                    })
                    .unwrap_or(true)
                    || gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
                        &repo,
                    ))
                    .map(|prefs| {
                        prefs.agent_roster.is_some_and(|roster| {
                            roster.members.iter().any(|member| {
                                member.assignment.as_ref().is_some_and(|assignment| {
                                    assignment.session_id.as_deref()
                                        == Some(pending.binding.session_id.as_str())
                                })
                            })
                        })
                    })
                    .unwrap_or(true)
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        if let Some(pty) = captured.pty.as_ref() {
            assert!(pty.tree_exit_confirmed());
        }
        if let Some(grant) = captured.grant.as_ref() {
            assert!(!captured.issuer.grant_is_current(grant));
        }
        let attempt = gwt::cli::execution_state::continuation_attempt_for_operation(
            &repo,
            authority,
            &operation_id,
        )
        .unwrap()
        .unwrap();
        if scenario == "activated-cancel" {
            assert_eq!(
                gwt::cli::execution_state::current_execution_binding(&repo, authority).unwrap(),
                Some(pending.binding.identity.clone())
            );
            assert_eq!(
                attempt.status,
                gwt::cli::execution_state::ContinuationAttemptStatus::Activated
            );
            let session = gwt_agent::Session::load(&candidate_path).unwrap();
            assert_eq!(session.execution_binding.as_ref(), Some(&pending.binding));
            assert_eq!(session.status, gwt_agent::AgentStatus::Stopped);
            assert!(!session.restore_window_on_startup);
        } else {
            assert_eq!(
                gwt::cli::execution_state::current_execution_binding(&repo, authority).unwrap(),
                Some(predecessor)
            );
            assert_eq!(
                attempt.status,
                gwt::cli::execution_state::ContinuationAttemptStatus::Aborted
            );
            assert!(!candidate_path.exists());
            assert!(
                !gwt_agent::runtime_state_path(&sessions, &pending.binding.session_id).exists()
            );
            assert!(!durable_launch_recovery_exists(
                &sessions,
                &pending.binding.session_id
            ));
            assert_eq!(
                gwt::cli::execution_state::load_generation_ledger(&repo, authority)
                    .unwrap()
                    .unwrap()
                    .generations
                    .len(),
                1
            );
        }
        assert_eq!(fs::read(&predecessor_path).unwrap(), predecessor_bytes);
        assert_eq!(tracked_work_event_store_snapshot(&repo), work_before);
        owner.shutdown().await;
    });
}

#[test]
fn fresh_execution_generic_close_aborts_candidate_in_background() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-generic-close-0",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        true,
        None,
    );
    let shared_owner = fixture.owner.clone();
    let predecessor = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join("predecessor-fresh-generic-close-0.toml"),
    )
    .unwrap();
    let predecessor_identity = gwt_agent::SessionExecutionIdentity::from_session(&predecessor)
        .unwrap()
        .unwrap();
    let tab = sample_project_tab("tab-1", "Repo", fixture.repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let baseline = runtime.window_scoped_state_entry_count();
    let root_pid = sysinfo::Pid::from_u32(std::process::id());
    let mut system = sysinfo::System::new();
    let mut rss_bytes = Vec::new();
    use gwt::project_runtime::{events::OwnerCommand, launch_payload::ManualSuccessorChoice};
    for round in 0..5 {
        if round > 0 {
            let operation_id = format!("fresh-generic-close-{round}");
            let executor = TokioRuntime::new().unwrap();
            let (captured, pending) = executor.block_on(async {
                let checkpoint = shared_owner.pause_next_agent_launch().await.unwrap();
                let command = owner_fresh_agent_command(
                    &shared_owner,
                    &fixture.repo,
                    ManualSuccessorChoice {
                        operation_id: operation_id.clone(),
                        expected_binding: fixture.predecessor_binding.clone(),
                        expected_predecessor: Some(Box::new(predecessor_identity.clone())),
                        expected_runtime: None,
                        predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor::Completed,
                    },
                    None,
                );
                let client = shared_owner.client();
                let launch = tokio::spawn(async move { client.command(command).await });
                // test-hygiene: allow-wall-clock-deadline hang guard for each actual successor preparation in the same Host
                let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
                    .await
                    .unwrap();
                let pending = captured.pending_fresh_execution.clone().unwrap();
                checkpoint.release();
                launch.await.unwrap().unwrap();
                (captured, pending)
            });
            fixture = OwnerFreshLaunchFixture {
                executor,
                owner: shared_owner.clone(),
                captured,
                pending,
                repo: fixture.repo.clone(),
                authority: fixture.authority,
                predecessor_binding: fixture.predecessor_binding.clone(),
            };
        }
        let raw = runtime
            .tab_mut("tab-1")
            .unwrap()
            .workspace
            .add_window(WindowPreset::Agent)
            .id;
        runtime.register_window("tab-1", &raw);
        let window_id = combined_window_id("tab-1", &raw);
        let context = runtime.test_context();
        let snapshot = fixture
            .executor
            .block_on(shared_owner.command(OwnerCommand::Snapshot {
                id: fixture.captured.id.clone(),
                incarnation: fixture.captured.incarnation,
            }))
            .unwrap();
        let probe = fixture
            .executor
            .block_on(shared_owner.probe(&fixture.captured.id, fixture.captured.incarnation))
            .unwrap();
        runtime.install_remote_owner_projection(
            &context,
            &window_id,
            shared_owner.client(),
            serde_json::from_value(snapshot).unwrap(),
            probe.session.clone(),
        );
        drain_queued_blocking_tasks(&tasks);
        let (entered, observed) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let _observer = gwt::project_runtime::agent_frontend::set_owner_agent_dispatch_test_hook(
            probe.grant.as_ref().unwrap(),
            gwt::project_runtime::agent_frontend::OwnerAgentDispatchTestStage::BeforeFreshWorkCommit,
            move || { entered.send(()).unwrap(); released.recv().unwrap(); });
        fixture.session_start(Some(fixture.pending.readiness_nonce.clone()));
        // test-hygiene: allow-wall-clock-deadline hang guard for actual readiness worker before queued physical close
        observed.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(runtime.close_window_outcome(&window_id).closed);
        let jobs = std::mem::take(&mut *tasks.lock().unwrap());
        let worker = thread::spawn(move || {
            for job in jobs {
                job();
            }
        });
        fixture.executor.block_on(async {
            // test-hygiene: allow-wall-clock-deadline hang guard for canonical close acceptance before releasing the exact readiness worker
            tokio::time::timeout(Duration::from_secs(30), async {
                while fixture.captured.current.load(Ordering::Acquire) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        });
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &fixture.repo,
                fixture.authority,
                &fixture.pending.operation_id
            )
            .unwrap()
            .unwrap()
            .status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
            "close acceptance cannot perform durable rollback"
        );
        assert!(probe.pty.try_wait().unwrap().is_none());
        release.send(()).unwrap();
        worker.join().unwrap();
        apply_recorded_window_close_finalized(&mut runtime, &recorded);
        assert_owner_fresh_launch_rolled_back(&fixture);
        assert_eq!(runtime.window_scoped_state_entry_count(), baseline);
        assert!(runtime.window_scoped_state_residue(&window_id).is_empty());
        let health = fixture
            .executor
            .block_on(shared_owner.command(OwnerCommand::Health))
            .unwrap();
        assert_eq!(
            health["test_pty_writers"], 0,
            "actual owner writer registry must release the failed candidate"
        );
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[root_pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing().with_memory(),
        );
        rss_bytes.push(system.process(root_pid).unwrap().memory());
    }
    eprintln!(
        "Issue #4959 AC-3: {}",
        serde_json::json!({
            "failed_launches": 5, "host_rss_bytes_after_cleanup": rss_bytes,
            "retained_window_entries": runtime.window_scoped_state_entry_count(), "retained_pty_writers": 0,
        })
    );
    let executor = TokioRuntime::new().unwrap();
    fixture.shutdown();
    executor.block_on(
        Arc::try_unwrap(shared_owner)
            .ok()
            .expect("all shared Host handles released")
            .shutdown(),
    );
}

#[test]
fn fresh_execution_generic_close_preserves_activated_or_replaced_candidate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    for activated in [false, true] {
        let _case_home = ScopedGwtHome::set(temp.path().join(if activated {
            "activated-home"
        } else {
            "replaced-home"
        }));
        let fixture = owner_fresh_launch_fixture(
            temp.path(),
            if activated {
                "close-activated"
            } else {
                "close-replaced"
            },
            gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            false,
            None,
        );
        let probe = fixture
            .executor
            .block_on(
                fixture
                    .owner
                    .probe(&fixture.captured.id, fixture.captured.incarnation),
            )
            .unwrap();
        let original = probe.session.as_ref().unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        if activated {
            gwt::cli::execution_state::activate_successor(
                &fixture.repo,
                fixture.authority,
                &fixture.pending.request,
            )
            .unwrap();
        } else {
            replace_fresh_candidate_session_incarnation(
                &sessions,
                &fixture.pending.binding.session_id,
            );
        }
        let path = sessions.join(format!("{}.toml", fixture.pending.binding.session_id));
        let before = fs::read(&path).unwrap();
        // The canonical retirement worker must preserve an irreversible
        // activation or a foreign exact Session before physical teardown.
        let outcome = gwt::project_runtime::launch_finalization::rollback_worker_fresh_execution(
            &sessions,
            &fixture.pending,
        );
        if activated {
            assert!(!outcome.unwrap());
        } else {
            assert!(outcome.is_err());
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(durable_launch_recovery_exists(
            &sessions,
            &fixture.pending.binding.session_id
        ));
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &fixture.repo,
                fixture.authority,
                &fixture.pending.operation_id
            )
            .unwrap()
            .unwrap()
            .status,
            if activated {
                gwt::cli::execution_state::ContinuationAttemptStatus::Activated
            } else {
                gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
            }
        );
        if !activated {
            original.save(&sessions).unwrap();
        }
        fixture.shutdown();
    }
}

#[test]
fn automatic_resume_successor_created_installs_active_authority_before_pty_spawn() {
    automatic_resume_capability_install(false);
}

#[test]
fn automatic_resume_capability_install_failure_reaches_unbound_pty() {
    automatic_resume_capability_install(true);
}

fn automatic_resume_capability_install(fail_install: bool) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    use gwt::project_runtime::{
        events::{OwnerCommand, OwnerEvent},
        test_support::OwnerLaunchStage,
    };
    let project = temp.path().join("actual-auto-resume");
    let bin = temp.path().join("actual-codex-bin");
    fs::create_dir_all(&bin).unwrap();
    let runner = bin.join(if cfg!(windows) { "codex.cmd" } else { "codex" });
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&runner, "#!/bin/sh\ncase \"$*\" in *--version*) printf '1.2.3\\n'; exit 0;; *'features list'*) exit 0;; esac\nstty -echo -icanon -isig; exec cat >/dev/null\n").unwrap();
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)] fs::write(&runner, "@echo off\r\nif \"%~1\"==\"--version\" (echo 1.2.3& exit /b 0)\r\nif \"%~1\"==\"features\" exit /b 0\r\npowershell -NoProfile -Command \"while ($null -ne [Console]::ReadLine()) {}\"\r\n").unwrap();
    let mut settings = Settings::default();
    let mut paths = vec![bin];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    settings
        .profiles
        .set_env_var(
            "default",
            "PATH",
            std::env::join_paths(paths).unwrap().to_str().unwrap(),
        )
        .unwrap();
    settings.save(&gwt_core::paths::gwt_config_path()).unwrap();
    let codex_home = temp.path().join("codex-home");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    pane_control_tests::control_init_repo(&project);
    seed_owner_fixture_roster(&project, 8);
    configure_owner_fixture_tools(&project, &["pane-contract-fixture", "codex"]);
    let (owner, bootstrap) = pane_control_tests::control_owner_agent(&project, "bootstrap", 900);
    let location = pane_control_tests::control_launch_config(&project, 42);
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(location.working_dir.unwrap())
        .branch(location.branch.unwrap())
        .base_branch(location.base_branch.unwrap())
        .build();
    config.command = runner.display().to_string();
    config.linked_issue_number = Some(42);
    config.explicit_follow_up = true;
    let predecessor = pane_control_tests::control_owner_launch(&owner, "predecessor", config);
    pane_control_tests::control_close(&owner, &bootstrap);
    let source_id = predecessor.session.as_ref().unwrap().id.clone();
    let predecessor_binding = predecessor
        .session
        .as_ref()
        .unwrap()
        .execution_binding
        .as_ref()
        .unwrap()
        .clone();
    // The real current producer accepts a self-close. Keep its direct ACK
    // armed across an external exact Stop, just as a delayed WebSocket can.
    let closing =
        fail_install.then(|| pane_control_tests::control_accept_self_close(&owner, &predecessor).0);
    pane_control_tests::control_close(&owner, &predecessor);
    assert!(predecessor.pty.tree_exit_confirmed());
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let path = sessions.join(format!("{source_id}.toml"));
    let mut source = gwt_agent::Session::load(&path).unwrap();
    let native_id = uuid::Uuid::new_v4().to_string();
    source.agent_session_id = Some(native_id.clone());
    source.save(&sessions).unwrap();
    let rollouts = codex_home.join("sessions");
    fs::create_dir_all(&rollouts).unwrap();
    fs::write(
        rollouts.join(format!("rollout-fixture-{native_id}.jsonl")),
        serde_json::json!({
            "type":"session_meta", "payload":{"id":native_id,"cwd":source.worktree_path}
        })
        .to_string(),
    )
    .unwrap();
    gwt::cli::execution_state::settle(
        &source.worktree_path,
        &source.id,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    )
    .unwrap();
    let config = gwt::session_launch::launch_config_from_persisted_session(&source);
    assert!(
        !config.automatic_restore,
        "this is the Automatic continuation intent of an explicit Resume, not a startup restore"
    );
    assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        let checkpoint = owner.pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall).await.unwrap();
        let mut events = owner.subscribe();
        let client = owner.client(); let id = owner.pane_id("resumed"); let target_id = id.clone();
        let launch = tokio::spawn(async move { client.command(OwnerCommand::LaunchAgent {
            id, operation_id: "automatic-resume-capability".into(), tab_id: "fixture".into(),
            config: Box::new(config), metadata: Box::default(), cols: 80, rows: 24,
        }).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual resume after capability installation and before PTY creation
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait()).await.unwrap();
        assert!(captured.pty.is_none(), "authority must be installed before any physical child exists");
        assert!(owner.probe(&target_id, captured.incarnation).await.is_err());
        let successor = captured.session.as_ref().expect("actual persisted resume Session");
        let grant = captured.grant.as_ref().expect("actual owner-issued resume capability");
        if fail_install {
            assert_ne!(successor.id, source_id);
            assert!(successor.execution_binding.is_none());
            assert!(!grant.principal().authorizes_producing_mutation());
            assert!(grant.principal().execution_binding().is_none());
            assert!(!gwt_agent::active_launch_handshake_path(&sessions, &successor.id).exists());
            let recorded = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
            assert!(recorded.iter().any(|event| matches!(event, OwnerEvent::PreparationOutput { data, .. }
                if String::from_utf8_lossy(data).contains("Resume degraded to unbound")
                    && String::from_utf8_lossy(data).contains("closing")
                    && String::from_utf8_lossy(data).contains("Start Work"))), "{recorded:?}");
            assert!(recorded.iter().any(|event| matches!(event, OwnerEvent::Notice { level, issue_number: Some(42), message }
                if level == "warn" && message.contains("closing"))), "{recorded:?}");
            assert!(gwt_agent::Session::load(&path).unwrap().execution_binding.is_some(), "retain exact coordinator authority for recovery");
        } else {
            let identity = gwt_agent::SessionExecutionIdentity::from_session(successor).unwrap().unwrap();
            assert_ne!(identity.execution_binding.identity.generation_id, predecessor_binding.identity.generation_id);
            assert_eq!(successor.linked_issue_number, Some(42));
            assert_eq!(gwt::cli::execution_state::current_execution_binding(&source.worktree_path,
                gwt::cli::execution_state::ExecutionOwnerKey { kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue, number: 42 }).unwrap(),
                Some(identity.execution_binding.identity.clone()));
            assert!(grant.principal().authorizes_producing_mutation());
            assert_eq!(grant.principal().active_execution_binding(), Some(&identity.execution_binding));
            assert!(gwt_agent::active_launch_handshake_path(&sessions, &successor.id).exists(), "the in-place Active relaunch is fenced before spawn");
        }
        checkpoint.release();
        let receipt = launch.await.unwrap().expect("automatic resume reaches actual PTY handoff");
        let live = owner.probe(&target_id, receipt["incarnation"].as_u64().unwrap()).await.unwrap();
        assert_eq!(live.session.as_ref().unwrap().id, successor.id);
        assert!(live.writer.is_some()); assert!(live.pty.try_wait().unwrap().is_none());
        let runtime = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(&sessions, &successor.id)).unwrap();
        assert_eq!(runtime.child_pid, live.pty.process_id());
        assert!(runtime.child_started_at.is_some_and(|birth| birth > 0));
        assert_eq!(runtime.execution_identity.as_ref(), gwt_agent::SessionExecutionIdentity::from_session(successor).unwrap().as_ref());
        owner.command(OwnerCommand::Input { id: target_id.clone(), incarnation: live.incarnation, data: b"resume input\r".to_vec() }).await.unwrap();
        if let Some(closing) = closing { drop(closing); owner.settled_self_closes().await.unwrap(); }
        owner.command(OwnerCommand::Close { id: target_id, incarnation: live.incarnation }).await.unwrap();
    });
}

#[cfg(unix)]
#[test]
fn manual_terminal_launch_persists_recovery_before_prepared_readiness() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for already_closed in [true, false] {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path());
        use gwt::project_runtime::{
            events::OwnerCommand,
            launch_payload::{ManualSuccessorChoice, OwnerLaunchMetadata},
            test_support::OwnerLaunchStage,
        };
        let project = temp.path().join("actual-manual-terminal");
        let (owner, predecessor) =
            pane_control_tests::control_owner_agent(&project, "predecessor", 3547);
        let prior = predecessor.session.as_ref().unwrap();
        let predecessor_identity = gwt_agent::SessionExecutionIdentity::from_session(prior)
            .unwrap()
            .unwrap();
        let predecessor_binding = predecessor_identity.execution_binding.identity.clone();
        let runtime_evidence = if already_closed {
            pane_control_tests::control_close(&owner, &predecessor);
            assert!(predecessor.pty.tree_exit_confirmed());
            match gwt::cli::execution_state::classify_exact_session_runtime(
                &gwt_core::paths::gwt_sessions_dir(),
                &predecessor_identity,
            )
            .unwrap()
            {
                gwt::cli::execution_state::ExactSessionRuntimeDisposition::Terminal(proof)
                | gwt::cli::execution_state::ExactSessionRuntimeDisposition::Defunct(proof) => {
                    gwt_agent::ManualLaunchRuntimeEvidence::Proof(proof)
                }
                gwt::cli::execution_state::ExactSessionRuntimeDisposition::Absent => {
                    gwt_agent::ManualLaunchRuntimeEvidence::Absent
                }
                other => panic!(
                    "actual canonical close must prove terminal or absent runtime: {other:?}"
                ),
            }
        } else {
            owner_fixture_live_runtime_evidence(&predecessor)
        };
        let mut config = pane_control_tests::control_launch_config(&project, 3547);
        config.explicit_follow_up = true;
        let operation_id = "manual-terminal-launch-operation";
        let metadata = OwnerLaunchMetadata {
            manual_successor: Some(ManualSuccessorChoice {
                operation_id: operation_id.into(),
                expected_binding: predecessor_binding.clone(),
                expected_predecessor: Some(Box::new(predecessor_identity.clone())),
                expected_runtime: Some(runtime_evidence),
                predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive,
            }),
            ..Default::default()
        };
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let executor = tokio::runtime::Runtime::new().unwrap();
        executor.block_on(async {
        let checkpoint = owner.pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall).await.unwrap();
        let client = owner.client(); let id = owner.pane_id("manual-successor");
        let mut launch = tokio::spawn(async move { client.command(OwnerCommand::LaunchAgent {
            id, operation_id: operation_id.into(), tab_id: "fixture".into(), config: Box::new(config), metadata: Box::new(metadata), cols: 80, rows: 24,
        }).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual manual terminal successor preparation before PTY spawn
        let captured = tokio::select! {
            result = &mut launch => panic!("exact terminal successor refused before materialization: {result:?}"),
            // test-hygiene: allow-wall-clock-deadline hang guard for the actual immutable terminal successor materialization
            captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait()) => captured.unwrap(),
        };
        assert!(predecessor.pty.tree_exit_confirmed(), "the owner must reap the exact predecessor before materializing readiness");
        if captured.session.is_none() {
            checkpoint.release();
            panic!("actual manual successor failed before materialization: {:?}", launch.await.unwrap());
        }
        let pending = captured.pending_fresh_execution.as_ref().unwrap();
        let source = captured.session.as_ref().unwrap();
        assert!(captured.pty.is_none());
        assert!(durable_launch_recovery_exists(&sessions, &source.id));
        assert!(!pending.readiness_nonce.is_empty());
        let binding = source.execution_binding.as_ref().unwrap();
        let grant = captured.grant.as_ref().unwrap();
        assert_eq!(grant.principal().execution_binding(), Some(binding));
        assert!(!grant.principal().authorizes_producing_mutation(), "the actual capability stays Prepared until readiness");
        let terminal_binding = gwt::cli::execution_state::current_execution_binding(&prior.worktree_path, pending.owner).unwrap().unwrap();
        assert_eq!(terminal_binding, pending.predecessor_binding);
        assert_eq!(terminal_binding.generation_id, predecessor_binding.generation_id);
        assert_eq!(terminal_binding.binding_id, predecessor_binding.binding_id,
            "readiness retains the predecessor after exact terminalization advances its ledger head");
        assert_eq!(gwt::cli::execution_state::continuation_attempt_for_operation(&prior.worktree_path, pending.owner, operation_id).unwrap().unwrap().status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Prepared);
        checkpoint.release(); let receipt = launch.await.unwrap().unwrap();
        let live = owner.probe(&captured.id, receipt["incarnation"].as_u64().unwrap()).await.unwrap();
        let actual = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(&sessions, &source.id)).unwrap();
        assert_eq!(actual.runtime_incarnation, Some(live.incarnation));
        assert_eq!(actual.child_pid, live.pty.process_id());
        assert_eq!(actual.execution_identity.as_ref(), gwt_agent::SessionExecutionIdentity::from_session(source).unwrap().as_ref());
        let request = gwt::AgentExecutionContinuationRequest { schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
            operation_id: uuid::Uuid::new_v4().to_string(), readiness_nonce: Some(pending.readiness_nonce.clone()) };
        let (status, result) = live.execution_continue(&request).await.unwrap();
        assert!(status.is_success(), "actual authenticated manual readiness: {result}");
        assert_eq!(gwt::cli::execution_state::current_execution_binding(&prior.worktree_path, pending.owner).unwrap(), Some(binding.identity.clone()));
        owner.command(OwnerCommand::Close { id: captured.id, incarnation: live.incarnation }).await.unwrap();
    });
    }
}

#[cfg(unix)]
#[test]
fn production_host_launch_all_runner_failure_leaves_no_session_or_success_dispatch() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-runner-failure");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&bin).expect("create bin");
    init_repo(&repo);
    let direct = bin.join("openclaw");
    fs::write(&direct, "#!/bin/sh\necho 'runner broken' >&2\nexit 1\n")
        .expect("write broken direct runner");
    fs::set_permissions(&direct, fs::Permissions::from_mode(0o755))
        .expect("chmod broken direct runner");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::OpenClaw)
        .working_dir(&repo)
        .branch("work/issue-2359")
        .session_mode(gwt_agent::SessionMode::Continue)
        .build();
    config.command = direct.display().to_string();
    config
        .env_vars
        .insert("PATH".to_string(), bin.display().to_string());
    config
        .env_vars
        .insert("HOME".to_string(), temp.path().display().to_string());
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45155/internal/hook-live",
        "ws://127.0.0.1:46255/ws",
        "ws://127.0.0.1:45155/internal/pane-ws",
    );
    let (proxy, events) = AppEventProxy::stub();

    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-runner-failure".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        Some(issuer),
    );

    let recorded = events.lock().expect("event log");
    let error = recorded
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchComplete { result, .. } => result.as_ref().as_ref().err(),
            _ => None,
        })
        .expect("failed LaunchComplete event");
    assert!(error.detail.contains("OpenClaw"), "{error}");
    assert!(error.detail.contains("exit status 1"), "{error}");
    assert!(recorded
        .iter()
        .all(|event| !matches!(recorded_project_payload(event), UserEvent::LaunchComplete { result, .. } if result.is_ok())));
    let persisted_sessions = fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    assert!(
        persisted_sessions.is_empty(),
        "runner-health failure must precede Session persistence: {persisted_sessions:?}"
    );
}

#[cfg(unix)]
#[test]
fn production_codex_health_failure_runs_once_before_managed_asset_or_session_mutation() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-codex-health-order");
    let bin = temp.path().join("bin");
    let invocation_counter = temp.path().join("codex-invocations.txt");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&bin).expect("create bin");
    init_repo(&repo);
    let direct = bin.join("codex");
    fs::write(
        &direct,
        "#!/bin/sh\nprintf 'probe\\n' >> \"${0%/*}/../codex-invocations.txt\"\n/bin/sleep 25\necho 'runner broken' >&2\nexit 1\n",
    )
    .expect("write broken Codex runner");
    fs::set_permissions(&direct, fs::Permissions::from_mode(0o755))
        .expect("chmod broken Codex runner");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&repo)
        .branch("work/issue-2359")
        .build();
    config.command = direct.display().to_string();
    config.env_vars.extend([
        ("PATH".to_string(), bin.display().to_string()),
        ("HOME".to_string(), temp.path().display().to_string()),
    ]);
    let (proxy, events) = AppEventProxy::stub();

    let started = std::time::Instant::now();
    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-codex-health-order".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        None,
    );
    let elapsed = started.elapsed();

    let recorded = events.lock().expect("event log");
    assert!(recorded
        .iter()
        .any(|event| matches!(recorded_project_payload(event), UserEvent::LaunchComplete { result, .. } if result.is_err())));
    assert_eq!(
        fs::read_to_string(&invocation_counter)
            .expect("Codex invocation counter")
            .lines()
            .count(),
        1,
        "canonical health must be the only Codex version invocation",
    );
    assert!(
        elapsed >= gwt_agent::DIRECT_RUNNER_PROBE_TIMEOUT - std::time::Duration::from_secs(1),
        "production launch must not return before the canonical probe deadline: {elapsed:?}",
    );
    assert!(
        !repo.join(".codex").exists(),
        "runner health must fail before managed Codex asset mutation",
    );
    assert!(
        fs::read_dir(&sessions_dir)
            .expect("read sessions dir")
            .flatten()
            .all(|entry| entry.path().extension().and_then(|ext| ext.to_str()) != Some("toml")),
        "runner health must fail before Session persistence",
    );
}

#[test]
fn failed_precommit_fresh_launch_does_not_orphan_its_persisted_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-precommit-failure");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        "blocked-precommit-session",
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            "blocked-precommit-session",
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "precommit regression fixture".to_string(),
                missing_verification: Some("fresh launch pending".to_string()),
            },
        )
        .expect("settle Blocked predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import Blocked predecessor");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&repo)
        .branch("work/issue-2359")
        .linked_issue_number(owner.number)
        .extra_arg("$gwt-execute #2359")
        .build();
    pin_config_fixture_runners(&mut config, temp.path(), &["codex", "npx", "bunx"]);
    let (proxy, events) = AppEventProxy::stub();

    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-precommit".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        None,
    );

    let launch_error = events
        .lock()
        .expect("event log")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchComplete { result, .. } => result.as_ref().as_ref().err().cloned(),
            _ => None,
        })
        .expect("failed LaunchComplete event");
    assert!(
        launch_error.detail.contains("Host capability issuer"),
        "expected Host capability issuer failure, got {launch_error:?}",
    );
    let persisted_candidates = fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    assert!(
        persisted_candidates.is_empty(),
        "a launch that never returned its Session id must not leave an undiscoverable candidate: {persisted_candidates:?}"
    );
    let recovery_receipts = fs::read_dir(sessions_dir.join("execution-launch-recovery"))
        .map(|entries| entries.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    assert!(
        recovery_receipts.is_empty(),
        "preflight failure must not publish a pending launch receipt: {recovery_receipts:?}",
    );
}

#[test]
fn genesis_final_runtime_persistence_failure_terminalizes_generation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-final-persist");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    fs::write(sessions_dir.join("runtime"), "not-a-directory")
        .expect("block runtime sidecar directory creation");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&repo)
        .branch("work/issue-2359")
        .linked_issue_number(owner.number)
        .extra_arg("$gwt-execute #2359")
        .build();
    pin_config_fixture_runners(&mut config, temp.path(), &["codex", "npx", "bunx"]);
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45155/internal/hook-live",
        "ws://127.0.0.1:46255/ws",
        "ws://127.0.0.1:45155/internal/pane-ws",
    );
    let (proxy, events) = AppEventProxy::stub();

    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-genesis-final-persist".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        Some(issuer),
    );

    let error = events
        .lock()
        .expect("event log")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchComplete { result, .. } => result.as_ref().as_ref().err().cloned(),
            _ => None,
        })
        .expect("failed LaunchComplete event");
    assert!(
        error.detail.contains("runtime") || error.detail.contains("directory"),
        "{error}"
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read failed genesis ledger")
        .expect("failed genesis ledger");
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    let persisted_sessions = fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    assert_eq!(
        persisted_sessions.len(),
        1,
        "cleanup I/O failure must retain exact Session evidence: {persisted_sessions:?}",
    );
    let retained_session_id = persisted_sessions[0]
        .path()
        .file_stem()
        .and_then(|value| value.to_str())
        .expect("retained Session id")
        .to_string();
    let recovery_dir = sessions_dir.join("execution-launch-recovery");
    let recovery_receipts = fs::read_dir(&recovery_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.path().extension().and_then(|ext| ext.to_str()) == Some("json")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(recovery_receipts.len(), 1, "{recovery_receipts:?}");
    assert!(
        recovery_dir
            .join(format!("{retained_session_id}.lock"))
            .exists(),
        "the durable receipt keeps its exact cross-process lock file"
    );

    fs::remove_file(sessions_dir.join("runtime")).expect("remove runtime path blocker");
    let mut restarted = sample_runtime(temp.path(), Vec::new(), None);
    reconcile_fixture_launches(&mut restarted, &repo);

    assert!(!sessions_dir
        .join(format!("{retained_session_id}.toml"))
        .exists());
    assert!(!durable_launch_recovery_exists(
        &sessions_dir,
        &retained_session_id,
    ));
}

#[test]
fn fresh_execution_prevalidation_conflict_leaves_work_writable_and_retains_candidate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-callback-failure",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    // This contract concerns the domain callback's exact request check. Use
    // the real owner-captured pending data and bearer, changing only that input.
    let mut pending = fixture.pending.clone();
    pending.request.principal_id = "mismatched-process-local-principal".into();
    let result = probe.with_agent_environment(|| {
        let token = std::env::var(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV).unwrap();
        gwt::project_runtime::continuation::finalize_fresh_execution_launch(
            &pending,
            probe.active_session.as_ref(),
            Some(&token),
            Some(&probe.issuer),
            Some(&pending.readiness_nonce),
            &gwt_core::paths::gwt_sessions_dir(),
            &gwt_core::paths::gwt_cache_dir(),
            &HashSet::from([pending.binding.session_id.clone()]),
            || probe.current.load(Ordering::Acquire),
        )
    });
    assert!(
        matches!(
            result,
            gwt::project_runtime::continuation::FreshExecutionFinalizationOutcome::Retained
        ),
        "callback refusal must retain its exact candidate until durable Aborted is proven"
    );
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.authority,
            &pending.operation_id
        )
        .unwrap()
        .unwrap()
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
    );
    let sessions = gwt_core::paths::gwt_sessions_dir();
    assert!(sessions
        .join(format!("{}.toml", pending.binding.session_id))
        .exists());
    assert!(durable_launch_recovery_exists(
        &sessions,
        &pending.binding.session_id
    ));
    let mut unrelated = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-unrelated-after-fresh-callback-failure",
        Utc::now(),
    );
    unrelated.title = Some("Unrelated writer after rejected callback".into());
    gwt_core::workspace_projection::record_workspace_work_event(&fixture.repo, unrelated)
        .expect("prevalidation refusal must not create a partial Work transaction");
    fixture.shutdown();
}

#[cfg(unix)]
#[test]
fn fresh_execution_launch_failure_retains_dangling_session_without_mutation() {
    use std::os::unix::fs::symlink;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-dangling-live",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let candidate_path = sessions.join(format!("{}.toml", fixture.pending.binding.session_id));
    fs::remove_file(&candidate_path).unwrap();
    let missing_target = sessions.join("missing-live-candidate");
    symlink(&missing_target, &candidate_path).unwrap();
    let receipt_path = sessions
        .join("execution-launch-recovery")
        .join(format!("{}.json", fixture.pending.binding.session_id));
    let receipt_before = fs::read(&receipt_path).unwrap();
    let authority_before = snapshot_optional_files(&exact_continue_authority_artifacts(
        &fixture.repo,
        fixture.authority,
    ));
    let work_before = tracked_workspace_work_store_snapshot(&fixture.repo);
    use gwt::project_runtime::events::OwnerCommand;
    let refused = fixture
        .executor
        .block_on(fixture.owner.command(OwnerCommand::Close {
            id: fixture.captured.id.clone(),
            incarnation: fixture.captured.incarnation,
        }));
    assert!(
        refused.is_err(),
        "unreadable exact Session must retain retirement evidence"
    );
    assert!(fs::symlink_metadata(&candidate_path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&candidate_path).unwrap(), missing_target);
    assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&fixture.repo, &work_before);
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.authority,
            &fixture.pending.operation_id
        )
        .unwrap()
        .unwrap()
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
    );
    // Missing is a different, provable state; the same exact owner retry now
    // completes the durable rollback instead of mutating the dangling entry.
    fs::remove_file(&candidate_path).unwrap();
    fixture
        .executor
        .block_on(fixture.owner.command(OwnerCommand::Close {
            id: fixture.captured.id.clone(),
            incarnation: fixture.captured.incarnation,
        }))
        .expect("exact Missing candidate cleanup retry");
    assert_owner_fresh_launch_rolled_back(&fixture);
    fixture.shutdown();
}

#[test]
fn fresh_execution_activated_response_loss_repairs_projection_pointer_and_workspace_commit() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _home_env = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
    let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-activated-repair",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    leave_owner_fresh_activated_before_projection_commit(&fixture);
    let probe = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    let grant = probe.current_capability_grant().unwrap();
    assert_eq!(
        probe.issuer.durable_authority(&grant),
        crate::embedded_server::AgentDurableAuthority::PendingActivation
    );
    let waiting = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    waiting.block_on(async {
        tokio::time::pause();
        let started = tokio::time::Instant::now();
        assert_eq!(
            gwt::agent_capability::durable_agent_execution_authority_async(
                grant.principal().clone()
            )
            .await,
            crate::embedded_server::AgentDurableAuthority::Unavailable,
            "unpublished activation must fail closed at the waiting deadline"
        );
        assert_eq!(started.elapsed().as_secs(), 10);
    });
    let (status, body) = fixture
        .executor
        .block_on(
            probe.execution_continue(&gwt::AgentExecutionContinuationRequest {
                schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: uuid::Uuid::new_v4().to_string(),
                readiness_nonce: None,
            }),
        )
        .unwrap();
    assert!(
        status.is_success(),
        "Activated reconciliation must publish normal completion before ACK: {status}: {body}"
    );
    assert_eq!(body["validated"], true);
    let completed = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(completed.readiness_watch.is_none());
    assert!(!completed.readiness_inflight);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.pending.binding.identity.clone())
    );
    assert_eq!(
        gwt_core::workspace_projection::resolve_workspace_state_external_commit(
            &fixture.repo,
            &fixture.pending.operation_id,
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit
        )
        .unwrap(),
        gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed
    );
    fixture.shutdown();
}

#[test]
fn continuation_resolves_legacy_mixed_root_external_commit_after_upgrade() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());

    for (label, decision, expected) in [
        (
            "reject",
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected,
        ),
        (
            "commit",
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
        ),
    ] {
        let project_root = temp.path().join(format!("legacy-mixed-{label}"));
        let work_event_root = project_root.join("work").join("issue-3412");
        fs::create_dir_all(&work_event_root).expect("legacy worktree root");
        let current = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
        let canonical_works =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        let legacy_works =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&work_event_root);
        let events = gwt_core::paths::gwt_repo_local_work_events_path(&work_event_root);
        let operation_id = format!("legacy-mixed-root-{label}");
        let now = Utc::now();
        let session_id = format!("legacy-mixed-session-{label}");
        let work_id = format!("legacy-mixed-work-{label}");
        let resume_event_id = format!("legacy-mixed-resume-{label}");
        let branch = "work/issue-3412";
        let container = gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some(branch.to_string()),
            worktree_path: Some(work_event_root.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };
        let mut current_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&project_root);
        current_projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: session_id.clone(),
                window_id: None,
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
                current_focus: None,
                title_summary: None,
                worktree_path: Some(work_event_root.clone()),
                branch: Some(branch.to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned,
                workspace_id: None,
                updated_at: now - chrono::Duration::seconds(2),
            });
        gwt_core::workspace_projection::save_workspace_projection_to_path(
            &current,
            &current_projection,
        )
        .expect("seed legacy current");

        let mut start = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            &work_id,
            now - chrono::Duration::seconds(2),
        );
        start.title = Some("Legacy mixed-root target".to_string());
        start.owner = Some("Issue #3412".to_string());
        start.status_category = Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Idle);
        start.agent_session_id = Some("legacy-predecessor".to_string());
        start.agent_id = Some("codex".to_string());
        start.execution_container = Some(container.clone());
        let mut legacy_projection =
            gwt_core::workspace_projection::WorkItemsProjection::empty(start.updated_at);
        legacy_projection.apply_event(start);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &legacy_works,
            &legacy_projection,
        )
        .expect("seed legacy worktree WorkItems");

        let mut canonical_projection = legacy_projection;
        let unrelated_work_id = format!("canonical-unrelated-{label}");
        let mut unrelated = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            &unrelated_work_id,
            now - chrono::Duration::seconds(1),
        );
        unrelated.title = Some("Existing canonical Work".to_string());
        canonical_projection.apply_event(unrelated);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &canonical_works,
            &canonical_projection,
        )
        .expect("seed pre-existing canonical WorkItems SOT");

        let prepared = gwt_core::workspace_projection::transact_workspace_state_at_with_commit(
            &current,
            &legacy_works,
            &events,
            &project_root,
            &operation_id,
            |projection, _, _| {
                assert!(projection.assign_agent(&session_id, &work_id, None, None, now));
                let mut resume = gwt_core::workspace_projection::WorkEvent::new(
                    gwt_core::workspace_projection::WorkEventKind::Resume,
                    &work_id,
                    now,
                );
                resume.id = resume_event_id.clone();
                resume.status_category =
                    Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
                resume.agent_session_id = Some(session_id.clone());
                resume.agent_id = Some("codex".to_string());
                resume.execution_container = Some(container.clone());
                Ok(((), vec![resume]))
            },
            || {
                Err(gwt_core::error::GwtError::Other(
                    "simulated legacy activation response loss".to_string(),
                ))
            },
        );
        assert!(prepared.is_err(), "legacy transaction must remain Prepared");

        assert_eq!(
            resolve_split_workspace_state_external_commit(
                &project_root,
                &work_event_root,
                &operation_id,
                decision,
            )
            .expect("new Host must resolve the legacy mixed-root transaction"),
            expected,
        );
        gwt_core::workspace_projection::transact_workspace_state_for_work_event_root(
            &project_root,
            &work_event_root,
            |_, _, _| Ok(((), Vec::new())),
        )
        .expect("resolved legacy transaction must not block the split-root writer");
        let canonical = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
            .expect("load canonical WorkItems")
            .expect("canonical WorkItems");
        assert!(
            canonical
                .work_items
                .iter()
                .any(|item| item.id == unrelated_work_id),
            "legacy recovery must preserve pre-existing canonical Works"
        );
        let target = canonical
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("canonical target Work");
        let has_resumed_session = target
            .agents
            .iter()
            .any(|agent| agent.session_id == session_id)
            && target
                .events
                .iter()
                .any(|event| event.id == resume_event_id);
        assert_eq!(
            has_resumed_session,
            expected
                == gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
            "only a committed legacy event may be folded into canonical WorkItems"
        );
    }
}

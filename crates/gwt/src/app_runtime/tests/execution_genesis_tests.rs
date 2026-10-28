use super::*;

pub(super) fn genesis_pre_pty_failure_fixture(
    temp: &Path,
) -> (
    Arc<gwt::project_runtime::test_support::ProjectOwnerHarness>,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
    gwt::project_runtime::events::OwnerLaunchFailure,
) {
    genesis_pre_pty_failure_fixture_with_command(temp, "/definitely/missing/pre-pty-provider")
}

pub(super) fn genesis_pre_pty_failure_fixture_with_command(
    temp: &Path,
    command: &str,
) -> (
    Arc<gwt::project_runtime::test_support::ProjectOwnerHarness>,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
    gwt::project_runtime::events::OwnerLaunchFailure,
) {
    let project = temp.join("actual-pre-pty-failure");
    let (owner, caller) = pane_control_tests::control_owner_agent(&project, "caller", 42);
    let mut config = pane_control_tests::control_launch_config(&project, 43);
    config.linked_issue_number = None;
    config.explicit_follow_up = false;
    config.command = command.into();
    let id = owner.pane_id("failed");
    let receipt = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::LaunchAgent {
            id: id.clone(),
            operation_id: "actual-pre-pty-failure".into(),
            tab_id: "actual-failure-tab".into(),
            config: Box::new(config),
            metadata: Box::default(),
            cols: 80,
            rows: 24,
        },
    ))
    .unwrap();
    let failure: gwt::project_runtime::events::OwnerLaunchFailure =
        serde_json::from_value(receipt["launch_failure"].clone()).unwrap();
    assert_eq!(failure.id, id);
    assert!(failure.detail.contains("pre-pty-provider"));
    assert!(
        owner_fixture_block_on(owner.probe(&id, failure.incarnation)).is_err(),
        "a diagnostic failure must not manufacture a physical runtime"
    );
    let source = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", failure.session_id)),
    )
    .unwrap();
    assert_eq!(source.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(
        gwt::project_runtime::events::launch_failure_session_fingerprint(&source).unwrap(),
        failure.session_fingerprint
    );
    (owner, caller, failure)
}

#[test]
fn app_runtime_agent_launch_completion_failure_emits_structured_error_log() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut expected = None;
    let events = capture_tracing_events(|| {
        let (owner, caller, failure) = genesis_pre_pty_failure_fixture(temp.path());
        expected = Some((
            failure.id.clone(),
            failure.session_id.clone(),
            failure.detail.clone(),
        ));
        pane_control_tests::control_close(&owner, &caller);
    });
    let (id, session, detail) = expected.unwrap();
    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("launch_complete")
                && event.fields.get("window_id").map(String::as_str) == Some(id.as_str())
        })
        .expect("the actual owner failure must emit its structured launch-complete log");
    assert_eq!(
        event.fields.get("tab_id").map(String::as_str),
        Some("actual-failure-tab")
    );
    assert_eq!(
        event.fields.get("raw_window_id").map(String::as_str),
        Some("failed")
    );
    assert_eq!(
        event.fields.get("session_id").map(String::as_str),
        Some(session.as_str())
    );
    assert_eq!(
        event.fields.get("error").map(String::as_str),
        Some(detail.as_str())
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("window_id").map(String::as_str) == Some(id.as_str()))
            .count(),
        1,
        "one actual failure has one canonical structured report"
    );
}

#[test]
fn app_runtime_agent_launch_completion_failure_writes_diagnostic_to_terminal() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (owner, caller, failure) = genesis_pre_pty_failure_fixture(temp.path());
    let reply = pane_control_tests::control_request(&owner, &caller, AgentFrontendRequest::Ready);
    let snapshot = reply
        .events
        .iter()
        .find(|event| event["kind"] == "terminal_snapshot" && event["id"] == failure.id)
        .unwrap_or_else(|| {
            panic!(
                "authenticated owner must present its retained failure diagnostic: {:?}",
                reply.events
            )
        });
    let diagnostic = base64::engine::general_purpose::STANDARD
        .decode(snapshot["data_base64"].as_str().unwrap())
        .unwrap();
    let diagnostic = String::from_utf8_lossy(&diagnostic);
    assert!(diagnostic.contains("Launch failed before PTY started"));
    assert!(diagnostic.contains(&failure.detail));
    let registry = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    assert!(registry["canonical_rows"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == "failed"
            && row["session_id"] == failure.session_id
            && row["status"] == "error"));
    pane_control_tests::control_close(&owner, &caller);
}

#[test]
fn stale_pre_pty_launch_failure_preserves_live_agent_and_monitor_delivery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    use gwt::project_runtime::{events::OwnerCommand, test_support::OwnerLaunchStage};
    let project = temp.path().join("project");
    let (owner, bootstrap) = pane_control_tests::control_owner_agent(&project, "bootstrap", 42);
    let mut failed = pane_control_tests::control_launch_config(&project, 43);
    failed.command = "/definitely/missing/stale-provider".into();
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        let checkpoint = owner
            .pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall)
            .await
            .unwrap();
        let client = owner.client();
        let id = owner.pane_id("target");
        let target = id.clone();
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::LaunchAgent {
                    id,
                    operation_id: "stale-pre-pty".into(),
                    tab_id: "fixture".into(),
                    config: Box::new(failed),
                    metadata: Box::default(),
                    cols: 80,
                    rows: 24,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual failed launch materialization checkpoint
        let stale = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        assert!(stale.session.is_some());
        assert!(stale.pty.is_none());
        checkpoint.release();
        let _actual_failure = launch.await.unwrap();
        let live = pane_control_tests::control_owner_launch(
            &owner,
            "target",
            pane_control_tests::control_launch_config(&project, 44),
        );
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project);
        gwt::mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            prefs.launched_issues.push(gwt::IssueMonitorLaunchedIssue {
                issue_number: 44,
                window_id: target.clone(),
            });
            prefs
                .launched_claims
                .insert(44, "current-live-claim".into());
        })
        .unwrap();
        let before = fs::read(&prefs_path).unwrap();
        owner
            .pane_event(gwt::pane_runtime::PaneRuntimeEvent::Status {
                id: stale.id.clone(),
                incarnation: stale.incarnation,
                status: WindowProcessStatus::Error,
                detail: Some("stale preparation failure".into()),
                exit_confirmed: false,
            })
            .await
            .unwrap();
        let _stale_cancel = owner
            .command(OwnerCommand::PendingCancel {
                id: stale.id,
                operation_id: stale.operation_id,
            })
            .await;
        let retained = owner.probe(&live.id, live.incarnation).await.unwrap();
        assert!(Arc::ptr_eq(&retained.pty, &live.pty));
        assert_eq!(
            retained.session.as_ref().unwrap().id,
            live.session.as_ref().unwrap().id
        );
        assert!(!matches!(
            retained.status,
            WindowProcessStatus::Stopped | WindowProcessStatus::Error
        ));
        assert!(retained
            .detail
            .as_deref()
            .is_none_or(|detail| !detail.contains("stale preparation")));
        assert_eq!(
            fs::read(&prefs_path).unwrap(),
            before,
            "stale failure must preserve the current Monitor binding/claim"
        );
        owner
            .command(OwnerCommand::Input {
                id: live.id.clone(),
                incarnation: live.incarnation,
                data: b"live-owner-input".to_vec(),
            })
            .await
            .unwrap();
        assert!(live.pty.has_unsent_user_input());
        pane_control_tests::control_close(&owner, &live);
    });
    pane_control_tests::control_close(&owner, &bootstrap);
}

#[test]
fn review_dispatch_launch_completion_keeps_the_implementation_binding() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    use gwt::project_runtime::{events::OwnerCommand, launch_payload::OwnerLaunchMetadata};
    let project = temp.path().join("project");
    pane_control_tests::control_init_repo(&project);
    seed_owner_fixture_roster(&project, 8);
    let owner = Arc::new(
        owner_fixture_block_on(
            gwt::project_runtime::test_support::ProjectOwnerHarness::start_with_monitor(&project),
        )
        .unwrap(),
    );
    let implementation = pane_control_tests::control_owner_launch(
        &owner,
        "implementation",
        pane_control_tests::control_launch_config(&project, 42),
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project);
    gwt::mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        prefs.enabled = true;
        prefs.autonomous_mode = true;
        prefs.launched_issues.push(gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: implementation.id.clone(),
        });
        prefs.launched_claims.insert(42, "claim-impl".into());
    })
    .unwrap();
    let before = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    let mut config = pane_control_tests::control_launch_config(&project, 44);
    config.linked_issue_number = Some(42);
    config.suppress_execution_control = true;
    config.is_ephemeral = true;
    config.branch = None;
    config
        .env_vars
        .insert("GWT_REVIEW_DISPATCH".into(), "1".into());
    let id = owner.pane_id("review");
    let receipt = owner_fixture_block_on(owner.command(OwnerCommand::LaunchAgent {
        id: id.clone(),
        operation_id: "review-dispatch".into(),
        tab_id: "fixture".into(),
        config: Box::new(config),
        metadata: Box::new(OwnerLaunchMetadata {
            launch_feedback_context: Some(LaunchFeedbackContext {
                client_id: "__issue_monitor__".into(),
                title: "Issue Monitor".into(),
                issue_monitor_issue_number: Some(42),
                issue_monitor_delivery_id: None,
                issue_monitor_project_root: Some(project.clone()),
                issue_monitor_session_mode: Some(gwt_agent::SessionMode::Normal),
                issue_monitor_autonomous_handoff: None,
                issue_monitor_autonomous_submit_started: false,
                issue_monitor_review_dispatch: true,
            }),
            ..Default::default()
        }),
        cols: 80,
        rows: 24,
    }))
    .unwrap();
    let review =
        owner_fixture_block_on(owner.probe(&id, receipt["incarnation"].as_u64().unwrap())).unwrap();
    assert!(review.session.as_ref().unwrap().review_dispatch);
    assert!(review.session.as_ref().unwrap().execution_binding.is_none());
    assert!(review.pty.try_wait().unwrap().is_none());
    let after = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert_eq!(after.launched_issues, before.launched_issues);
    assert_eq!(after.launched_claims, before.launched_claims);
    assert_eq!(after.requeue_audit, before.requeue_audit);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(
            &implementation.session.as_ref().unwrap().worktree_path,
            gwt::cli::execution_state::ExecutionOwnerKey {
                kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 42,
            }
        )
        .unwrap(),
        implementation
            .session
            .as_ref()
            .unwrap()
            .execution_binding
            .as_ref()
            .map(|binding| binding.identity.clone())
    );
    pane_control_tests::control_close(&owner, &review);
    pane_control_tests::control_close(&owner, &implementation);
}

#[test]
fn genesis_pty_spawn_failure_terminalizes_generation_and_allows_successor_retry() {
    assert_genesis_pty_failure_recovers_member(false);
}

#[test]
fn genesis_roster_failure_retains_receipt_until_startup_retry_recovers() {
    assert_genesis_pty_failure_recovers_member(true);
}

fn assert_genesis_pty_failure_recovers_member(roster_read_fails: bool) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    use gwt::project_runtime::{events::OwnerCommand, test_support::OwnerLaunchStage};
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    init_repo_with_initial_commit(&project);
    seed_owner_fixture_roster(&project, 8);
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        let owner = production_owner_fixture(&project).await;
        let checkpoint = owner
            .pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall)
            .await
            .unwrap();
        let mut config = pane_control_tests::control_launch_config(&project, 2359);
        config.command = "/definitely/missing/gwt-agent".into();
        let client = owner.client();
        let id = owner.pane_id("genesis-failure");
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::LaunchAgent {
                    id,
                    operation_id: "genesis-pty-failure".into(),
                    tab_id: "fixture".into(),
                    config: Box::new(config),
                    metadata: Box::default(),
                    cols: 80,
                    rows: 24,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual Genesis materialization checkpoint
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        let source = captured.session.as_ref().unwrap();
        let binding = source.execution_binding.as_ref().unwrap();
        let member_id = source.agent_member_id.as_ref().unwrap();
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project);
        let member_before = gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .agent_roster
            .unwrap()
            .members
            .into_iter()
            .find(|member| &member.agent_member_id == member_id)
            .unwrap();
        let authority = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 2359,
        };
        assert!(captured.pty.is_none());
        let prefs_backup = temp.path().join("genesis-roster-backup.json");
        if roster_read_fails {
            fs::rename(&prefs_path, &prefs_backup).unwrap();
            fs::create_dir(&prefs_path).unwrap();
            assert!(gwt::load_issue_monitor_prefs(&prefs_path).is_err());
        }
        checkpoint.release();
        let result = launch.await.unwrap();
        let inventory = owner.command(OwnerCommand::ListPanes).await.unwrap();
        assert!(inventory["panes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pane| pane["id"] != captured.id));
        assert!(
            result.is_err()
                || result
                    .as_ref()
                    .unwrap()
                    .get("compensated_launch_failure")
                    .is_some(),
            "{result:?}"
        );
        assert_eq!(
            gwt::cli::execution_state::load_generation_ledger(&source.worktree_path, authority)
                .unwrap()
                .unwrap()
                .current_effective_status(),
            Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
        );
        let sessions = gwt_core::paths::gwt_sessions_dir();
        assert!(!sessions.join(format!("{}.toml", source.id)).exists());
        let retained_after_failure = durable_launch_recovery_exists(&sessions, &source.id);
        owner.shutdown().await;
        let mut retained_during_startup_fault = None;
        if roster_read_fails {
            let failed_retry = production_owner_fixture(&project).await;
            retained_during_startup_fault =
                Some(durable_launch_recovery_exists(&sessions, &source.id));
            failed_retry.shutdown().await;
            fs::remove_dir(&prefs_path).unwrap();
            fs::rename(&prefs_backup, &prefs_path).unwrap();
            let normal_retry = production_owner_fixture(&project).await;
            normal_retry.shutdown().await;
        }
        // Read persisted prefs directly: roster recovery must not repair the result.
        let member_after = gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .agent_roster
            .unwrap()
            .members
            .into_iter()
            .find(|member| &member.agent_member_id == member_id)
            .unwrap();
        assert!(
            gwt_core::workspace_projection::load_workspace_work_items(&project)
                .unwrap()
                .is_none_or(|items| items.work_items.is_empty())
        );
        let terminal_binding =
            gwt::cli::execution_state::current_execution_binding(&source.worktree_path, authority)
                .unwrap()
                .unwrap();
        assert_eq!(
            terminal_binding.generation_id,
            binding.identity.generation_id
        );
        assert_eq!(terminal_binding.binding_id, binding.identity.binding_id);
        let request = gwt::cli::execution_state::SuccessorRequest {
            operation_id: "genesis-pty-retry".into(),
            principal_id: "gwt-host-launch".into(),
            work_id: None,
            source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.into(),
            session_binding_id: "genesis-pty-retry-binding".into(),
            initial_session_id: "genesis-pty-retry-session".into(),
            entrypoint: "$gwt-execute #2359".into(),
            requested_at: Utc::now(),
        };
        gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
            &source.worktree_path,
            authority,
            &request,
        )
        .expect("terminal Genesis permits a real successor request");
        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(&source.worktree_path, authority)
                .unwrap(),
            Some(terminal_binding)
        );
        assert_eq!(retained_after_failure, roster_read_fails);
        if roster_read_fails {
            assert_eq!(
                retained_during_startup_fault,
                Some(true),
                "roster recovery failure must retain its durable retry receipt"
            );
        }
        assert!(!durable_launch_recovery_exists(&sessions, &source.id));
        let assignment = member_before.assignment.as_ref().unwrap();
        assert!(!assignment.reservation_id.is_empty());
        assert_eq!(assignment.session_id.as_deref(), Some(source.id.as_str()));
        assert!(
            member_after.assignment.is_none(),
            "failed genesis must not strand its member assignment"
        );
        assert_eq!(member_after.name, member_before.name);
        assert_eq!(member_after.enabled, member_before.enabled);
        assert_eq!(member_after.agent_id, member_before.agent_id);
    });
}

#[test]
fn genesis_receipt_cleanup_failure_discards_published_work_and_active_owner() {
    assert_genesis_receipt_cleanup_failure(false, false);
}

#[test]
fn genesis_receipt_cleanup_failure_evicts_cached_session_after_window_close() {
    assert_genesis_receipt_cleanup_failure(false, true);
}

#[test]
fn genesis_receipt_cleanup_failure_evicts_cached_session_when_window_already_closed() {
    assert_genesis_receipt_cleanup_failure(true, false);
}

#[test]
fn ordinary_blocked_transition_is_not_genesis_failure_compensation_authority() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-ordinary-blocked-genesis-receipt");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "ordinary-blocked-session";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize generation ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read binding")
        .expect("current binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let context = WorkspaceResumeContext {
        title: Some("Ordinary blocked Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: None,
        next_action: None,
    };
    let mut session = sample_active_agent_session("tab-1", "window-ordinary-blocked");
    session.session_id = session_id.to_string();
    session.branch_name = "work/issue-2359".to_string();
    session.worktree_path = repo.clone();
    session.agent_project_root = repo.display().to_string();
    save_workspace_launch_projection(
        &repo,
        &session,
        Some("develop"),
        Some(owner.number),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([session.session_id.clone()])),
    )
    .expect("publish Work");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            session_id,
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "ordinary agent blocker".to_string(),
                missing_verification: Some("ordinary verification gap".to_string()),
            },
        )
        .expect("settle ordinary blocker"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));

    assert!(
        compensate_terminalized_genesis_workspace_projection(
            &repo,
            &repo,
            owner,
            session_id,
            Some(&binding),
            Some(&session.branch_name),
        )
        .is_err(),
        "ordinary execution.blocked must not authorize failed-genesis Work compensation",
    );
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read WorkItems")
        .expect("WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    assert!(!work_items.work_items[0].discarded);
}

#[test]
fn terminalized_genesis_compensation_uses_repo_global_work_items() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let project_root = temp.path().join("workspace-home");
    let (bare, _develop_worktree) = init_managed_workspace_with_develop_worktree(&project_root);
    let worktree = project_root.join("work").join("issue-3412");
    fs::create_dir_all(worktree.parent().expect("worktree parent"))
        .expect("create worktree parent");
    run_git(
        &bare,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "work/issue-3412",
            worktree.to_str().expect("worktree path"),
            "develop",
        ],
    );
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3412,
    };
    let session_id = "genesis-split-root-session";
    gwt::cli::execution_state::materialize_at_launch(
        &worktree,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #3412",
        false,
    )
    .expect("materialize execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &worktree,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize generation ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&worktree, owner)
        .expect("read binding")
        .expect("current binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&worktree).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let context = WorkspaceResumeContext {
        title: Some("Failed genesis split-root Work".to_string()),
        owner: Some("Issue #3412".to_string()),
        summary: None,
        next_action: None,
    };
    let mut session = sample_active_agent_session("tab-1", "window-genesis-split-root");
    session.session_id = session_id.to_string();
    session.branch_name = "work/issue-3412".to_string();
    session.worktree_path = worktree.clone();
    session.agent_project_root = project_root.display().to_string();
    save_workspace_launch_projection(
        &project_root,
        &session,
        Some("develop"),
        Some(owner.number),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([session.session_id.clone()])),
    )
    .expect("publish split-root Work");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &worktree,
        owner,
        session_id,
        &binding.identity,
        "test split-root genesis compensation",
    )
    .expect("terminalize failed genesis");

    compensate_terminalized_genesis_workspace_projection(
        &project_root,
        &worktree,
        owner,
        session_id,
        Some(&binding),
        Some(&session.branch_name),
    )
    .expect("compensate split-root Work");

    // Issue #4674: a fresh intake must retain the machine-local close even
    // though shared lifecycle sources deliberately exclude close events.
    let intake = crate::work_events_ingest::ingest_project_work_events_paths(
        &project_root,
        &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
        &gwt_core::paths::gwt_workspace_work_events_intake_state_path_for_repo_path(&project_root),
    );
    assert!(
        intake.projection_rebuilt,
        "fresh intake must rebuild: {intake:?}"
    );

    let current = gwt_core::workspace_projection::load_workspace_projection(&project_root)
        .expect("read current")
        .expect("current");
    assert!(current.latest_agent_for_session(session_id).is_none());
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
        .expect("read repo-global WorkItems")
        .expect("repo-global WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    assert!(work_items.work_items[0].discarded);
    assert_eq!(
        materialized_project_state_sots("works.json"),
        vec![gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root)],
        "terminalization must leave exactly one repository WorkItems SOT"
    );
    assert!(load_tracked_work_events(&worktree)
        .iter()
        .any(|event| event.agent_session_id.as_deref() == Some(session_id)));
    assert!(!load_tracked_work_events(&worktree)
        .iter()
        .any(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Discard));
    let closes = fs::read_to_string(
        gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&project_root),
    )
    .expect("read machine-local genesis close log");
    assert!(closes.lines().any(|line| {
        let event: gwt_core::workspace_projection::WorkEvent =
            serde_json::from_str(line).expect("decode close event");
        event.kind == gwt_core::workspace_projection::WorkEventKind::Discard
            && event.agent_session_id.as_deref() == Some(session_id)
    }));
}

#[test]
fn terminalized_genesis_compensation_pauses_instead_of_discarding_resumed_work() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-resume-compensation");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let failed_session_id = "genesis-failed-resume";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        failed_session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read genesis binding")
        .expect("genesis binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: failed_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let context = WorkspaceResumeContext {
        title: Some("Existing Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: Some("Keep this Work after a failed resume".to_string()),
        next_action: Some("Continue from the prior conversation".to_string()),
    };
    let mut prior = sample_active_agent_session("tab-1", "window-prior");
    prior.session_id = "prior-session".to_string();
    prior.branch_name = "work/issue-2359".to_string();
    prior.worktree_path = repo.clone();
    prior.agent_project_root = repo.display().to_string();
    save_workspace_launch_projection(
        &repo,
        &prior,
        Some("develop"),
        Some(owner.number),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([prior.session_id.clone()])),
    )
    .expect("publish existing Work");
    let existing = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read existing WorkItems")
        .expect("existing WorkItems")
        .work_items
        .into_iter()
        .next()
        .expect("existing Work");
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        &existing.id,
        Some(&existing.title),
        existing.summary.as_deref(),
        existing.owner.as_deref(),
        &existing.board_refs,
        existing.execution_containers.first().cloned(),
        Some(&prior.session_id),
        Utc::now(),
    )
    .expect("pause prior Session");
    gwt_core::workspace_projection::mark_workspace_agent_stopped(&repo, &prior.session_id, None)
        .expect("remove prior Session");

    let mut failed = prior.clone();
    failed.window_id = "window-failed".to_string();
    failed.session_id = failed_session_id.to_string();
    save_resumed_workspace_projection(
        &repo,
        &failed,
        None,
        Some(owner.number),
        Some(owner),
        &context,
        Some(&HashSet::from([failed.session_id.clone()])),
    )
    .expect("publish failed resume attempt");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &repo,
        owner,
        failed_session_id,
        &binding.identity,
        "test failed resume compensation",
    )
    .expect("terminalize failed genesis");
    let successor = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "genesis-late-cleanup-successor".to_string(),
        principal_id: "gwt-host-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "genesis-late-cleanup-binding".to_string(),
        initial_session_id: "genesis-late-cleanup-session".to_string(),
        entrypoint: "$gwt-execute #2359".to_string(),
        requested_at: Utc::now(),
    };
    gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
        &repo, owner, &successor,
    )
    .expect("prepare successor before late genesis cleanup");
    gwt::cli::execution_state::activate_successor(&repo, owner, &successor)
        .expect("activate successor before late genesis cleanup");
    assert_ne!(
        gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read successor binding"),
        Some(binding.identity.clone()),
        "the genesis generation must be historical before compensation",
    );

    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        failed_session_id,
        Some(&binding),
        Some(&failed.branch_name),
    )
    .expect("compensate failed resume without deleting existing Work");
    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        failed_session_id,
        Some(&binding),
        Some(&failed.branch_name),
    )
    .expect("retry after Pause and exact agent cleanup");

    let intake = crate::work_events_ingest::ingest_project_work_events_paths(
        &repo,
        &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo),
        &gwt_core::paths::gwt_workspace_work_events_intake_state_path_for_repo_path(&repo),
    );
    assert!(
        intake.projection_rebuilt,
        "fresh intake must rebuild: {intake:?}"
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("read compensated Workspace")
        .expect("compensated Workspace");
    assert!(projection
        .latest_agent_for_session(failed_session_id)
        .is_none());
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read compensated WorkItems")
        .expect("compensated WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    let retained = &work_items.work_items[0];
    assert_eq!(retained.id, existing.id);
    assert!(
        !retained.discarded,
        "a resumed Work must never be discarded"
    );
    assert_eq!(
        retained.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
    );
    assert!(retained.events.iter().any(|event| {
        event.kind == gwt_core::workspace_projection::WorkEventKind::Resume
            && event.agent_session_id.as_deref() == Some(failed_session_id)
    }));
    assert!(retained.events.iter().any(|event| {
        event.kind == gwt_core::workspace_projection::WorkEventKind::Pause
            && event.agent_session_id.as_deref() == Some(failed_session_id)
    }));
    assert!(!load_tracked_work_events(&repo)
        .iter()
        .any(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Pause));
    let closes = fs::read_to_string(
        gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&repo),
    )
    .expect("read machine-local resume close log");
    assert!(closes.lines().any(|line| {
        let event: gwt_core::workspace_projection::WorkEvent =
            serde_json::from_str(line).expect("decode close event");
        event.kind == gwt_core::workspace_projection::WorkEventKind::Pause
            && event.agent_session_id.as_deref() == Some(failed_session_id)
    }));
}

#[test]
fn terminalized_genesis_compensation_retries_after_discard_before_agent_cleanup() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-discard-retry");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-discard-response-loss";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read genesis binding")
        .expect("genesis binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let context = WorkspaceResumeContext {
        title: Some("New failed Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: None,
        next_action: None,
    };
    let mut session = sample_active_agent_session("tab-1", "window-discard-retry");
    session.session_id = session_id.to_string();
    session.branch_name = "work/issue-2359".to_string();
    session.worktree_path = repo.clone();
    session.agent_project_root = repo.display().to_string();
    save_workspace_launch_projection(
        &repo,
        &session,
        Some("develop"),
        Some(owner.number),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([session.session_id.clone()])),
    )
    .expect("publish failed new Work");
    let published_projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("read published Workspace")
        .expect("published Workspace");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &repo,
        owner,
        session_id,
        &binding.identity,
        "test discard response loss",
    )
    .expect("terminalize failed genesis");
    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        session_id,
        Some(&binding),
        Some(&session.branch_name),
    )
    .expect("discard new Work");
    gwt_core::workspace_projection::save_workspace_projection(&repo, &published_projection)
        .expect("restore pre-agent-cleanup projection after simulated response loss");

    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        session_id,
        Some(&binding),
        Some(&session.branch_name),
    )
    .expect("retry exact agent cleanup after durable Discard");
    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        session_id,
        Some(&binding),
        Some(&session.branch_name),
    )
    .expect("retry after Discard and exact agent cleanup");

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("read retried Workspace")
        .expect("retried Workspace");
    assert!(projection.latest_agent_for_session(session_id).is_none());
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read retried WorkItems")
        .expect("retried WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    assert!(work_items.work_items[0].discarded);
    assert_eq!(
        work_items.work_items[0]
            .events
            .iter()
            .filter(|event| {
                event.kind == gwt_core::workspace_projection::WorkEventKind::Discard
            })
            .count(),
        1,
        "retry must not append a duplicate Discard event",
    );
}

#[test]
fn startup_genesis_receipt_imports_flat_authority_and_terminalizes_crash_window() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-flat-crash-window";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize flat genesis authority");
    assert!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read absent genesis ledger")
            .is_none()
    );
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::Genesis,
        session_id,
        &repo,
        &repo,
        owner,
        None,
        None,
    )
    .expect("persist genesis crash receipt");

    reconcile_fixture_launches(&mut runtime, &repo);

    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read recovered genesis ledger")
        .expect("recovered genesis ledger");
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
}

#[cfg(unix)]
#[test]
fn startup_genesis_recovery_retains_dangling_session_entry_without_mutation() {
    use std::os::unix::fs::symlink;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-dangling-session");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-dangling-session";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read genesis binding")
        .expect("genesis binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(repo.clone());
    session.linked_issue_number = Some(owner.number);
    session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind genesis Session");
    session
        .save(&runtime.sessions_dir)
        .expect("save genesis Session");
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::Genesis,
        session_id,
        &repo,
        &repo,
        owner,
        Some(&binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("persist genesis receipt");
    let session_path = runtime.sessions_dir.join(format!("{session_id}.toml"));
    fs::remove_file(&session_path).expect("remove Session before dangling entry");
    let dangling_target = runtime.sessions_dir.join("missing-session-target");
    symlink(&dangling_target, &session_path).expect("create dangling Session entry");
    let receipt_path = runtime
        .sessions_dir
        .join("execution-launch-recovery")
        .join(format!("{session_id}.json"));
    let receipt_before = fs::read(&receipt_path).expect("read recovery receipt");
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
    let work_before = tracked_workspace_work_store_snapshot(&repo);

    reconcile_fixture_launches(&mut runtime, &repo);

    assert!(fs::symlink_metadata(&session_path)
        .expect("dangling Session entry must remain")
        .file_type()
        .is_symlink(),);
    assert_eq!(fs::read_link(&session_path).unwrap(), dangling_target);
    assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&repo, &work_before);

    fs::remove_file(&session_path).expect("restore genuinely missing genesis Session");
    let sessions_dir = runtime.sessions_dir.clone();
    let replacement_repo = repo.clone();
    set_missing_session_cleanup_hook_for_test(Box::new(move |observed_session_id| {
        assert_eq!(observed_session_id, session_id);
        let mut replacement = gwt_agent::Session::new(
            &replacement_repo,
            "work/replacement",
            gwt_agent::AgentId::Codex,
        );
        replacement.id = observed_session_id.to_string();
        replacement
            .save(&sessions_dir)
            .expect("materialize same-id genesis Session after Missing observation");
    }));

    reconcile_fixture_launches(&mut runtime, &repo);

    assert!(
        session_path.exists(),
        "replacement genesis Session must remain"
    );
    assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&repo, &work_before);

    fs::remove_file(&session_path).expect("restore true Missing genesis state");
    reconcile_fixture_launches(&mut runtime, &repo);
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read terminalized genesis ledger")
            .expect("terminalized genesis ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
}

#[test]
fn startup_repairs_ledger_first_genesis_terminalization_before_cleanup() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-ledger-first-restart");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-ledger-first-restart";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read genesis binding")
        .expect("genesis binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(repo.clone());
    session.linked_issue_number = Some(owner.number);
    session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind genesis Session");
    session
        .save(&runtime.sessions_dir)
        .expect("save genesis Session");
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::Genesis,
        session_id,
        &repo,
        &repo,
        owner,
        Some(&binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("persist genesis receipt");
    let session_path = runtime.sessions_dir.join(format!("{session_id}.toml"));
    let mut replacement = session.clone();
    replacement.agent_id = gwt_agent::AgentId::Custom("review-bot".to_string());
    replacement
        .save(&runtime.sessions_dir)
        .expect("replace genesis Session Agent");
    let replacement_before = fs::read(&session_path).expect("read replacement Session");
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
    let work_before = tracked_workspace_work_store_snapshot(&repo);

    reconcile_fixture_launches(&mut runtime, &repo);

    assert_eq!(
        fs::read(&session_path).expect("read retained replacement Session"),
        replacement_before,
    );
    assert!(durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&repo, &work_before);
    session
        .save(&runtime.sessions_dir)
        .expect("restore exact genesis Session");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &repo,
        owner,
        session_id,
        &binding.identity,
        "simulated launch failure",
    )
    .expect("commit terminal ledger event");
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(&repo)
        .expect("trusted worktree directory");
    fs::remove_file(trusted_dir.join("execution-control.json"))
        .expect("remove committed projection");
    fs::remove_file(trusted_dir.join("execution-generation-pointer.json"))
        .expect("remove committed pointer");
    assert!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner).is_err(),
        "fixture must reproduce the ledger-first unreadable strict view",
    );

    reconcile_fixture_launches(&mut runtime, &repo);

    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read repaired authority")
            .expect("repaired ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    assert!(!runtime
        .sessions_dir
        .join(format!("{session_id}.toml"))
        .exists());
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
}

#[test]
fn continue_work_launch_failure_aborts_without_pausing_candidate_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (_runtime, repo, authority, selected_work_id) =
        owner_continuation_seed(temp.path(), "continue-source-failure");
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        use gwt::project_runtime::{events::OwnerCommand, test_support::OwnerLaunchStage};
        let owner = production_owner_fixture(&repo).await;
        let checkpoint = owner
            .pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall)
            .await
            .unwrap();
        let client = owner.client();
        let work_id = selected_work_id.clone();
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::ContinueWork {
                    client_id: "client".into(),
                    operation_id: "continue-op-1".into(),
                    work_id,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual ContinueWork candidate materialization
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        assert!(captured.pty.is_none());
        let pending = captured.pending_continue_work.as_ref().unwrap();
        let source = captured.session.as_ref().unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let path = sessions.join(format!("{}.toml", source.id));
        let work_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
        let work_before = fs::read(&work_path).unwrap();
        let events_before = tracked_work_event_store_snapshot(&repo);
        let authority_before =
            snapshot_optional_files(&exact_continue_authority_artifacts(&repo, authority));
        let mut replacement = source.clone();
        replacement.branch = "work/replacement-race".into();
        replacement.save(&sessions).unwrap();
        let replacement_before = fs::read(&path).unwrap();
        let changed = gwt::project_runtime::launch_finalization::rollback_worker_continue_work(
            &sessions, pending,
        );
        let replacement_after = fs::read(&path).unwrap();
        source.save(&sessions).unwrap();
        assert!(
            changed.is_err(),
            "a changed candidate must refuse exact rollback"
        );
        assert_eq!(replacement_before, replacement_after);
        #[cfg(unix)]
        {
            fs::remove_file(&path).unwrap();
            let missing = sessions.join("missing-live-continue-candidate");
            std::os::unix::fs::symlink(&missing, &path).unwrap();
            let dangling = gwt::project_runtime::launch_finalization::rollback_worker_continue_work(
                &sessions, pending,
            );
            let still_symlink = fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink();
            let retained_target = fs::read_link(&path).unwrap();
            fs::remove_file(&path).unwrap();
            source.save(&sessions).unwrap();
            assert!(dangling.is_err());
            assert!(still_symlink);
            assert_eq!(retained_target, missing);
        }
        assert_eq!(fs::read(&work_path).unwrap(), work_before);
        assert_eq!(tracked_work_event_store_snapshot(&repo), events_before);
        assert_optional_files_unchanged(&authority_before);
        let client = owner.client();
        let id = captured.id.clone();
        let operation_id = captured.operation_id.clone();
        let cancel = tokio::spawn(async move {
            client
                .command(OwnerCommand::PendingCancel { id, operation_id })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for actual cancellation admission before releasing the preparation worker
        tokio::time::timeout(Duration::from_secs(30), async {
            while captured.current.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        checkpoint.release();
        assert!(launch.await.unwrap().is_err());
        cancel.await.unwrap().unwrap();
        assert!(!path.exists());
        assert!(!gwt_agent::runtime_state_path(&sessions, &source.id).exists());
        let work = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .unwrap()
            .unwrap();
        assert!(work
            .work_items
            .iter()
            .all(|item| item.id != format!("work-session-{}", source.id)));
        assert_eq!(
            work.work_items
                .iter()
                .find(|item| item.id == selected_work_id)
                .unwrap()
                .status_category,
            gwt_core::workspace_projection::WorkspaceStatusCategory::Done
        );
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &repo,
                authority,
                &pending.operation_id
            )
            .unwrap()
            .unwrap()
            .status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Aborted
        );
        owner.shutdown().await;
    });
}

#[test]
fn continue_work_ready_timeout_is_correlated_and_aborts_only_current_pending_attempt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "current-readiness-operation",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    use gwt::project_runtime::events::OwnerCommand;
    let before = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(fixture
        .executor
        .block_on(fixture.owner.check_readiness_deadline(
            &fixture.captured.id,
            fixture.captured.incarnation,
            ContinueWorkReadinessWatch::new("stale-operation".into())
        ))
        .is_err());
    let retained = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert_eq!(retained.readiness_watch, before.readiness_watch);
    assert!(Arc::ptr_eq(&retained.pty, &before.pty));
    gwt::session_finalizer::kill_and_reap(&before.pty, &fixture.captured.id)
        .expect("actual candidate process exit");
    assert!(before.pty.tree_exit_confirmed());
    assert!(matches!(
        continue_work_readiness_decision(
            &ContinueWorkReadinessWatch::new(fixture.pending.operation_id.clone()),
            ReadinessPaneEvidence::Dead,
            0
        ),
        ReadinessDeadlineDecision::Abort {
            pane: gwt::project_runtime::launch_readiness::LaunchPaneDisposition::Teardown,
            ..
        }
    ));
    // The real process is terminal. Its owner may already have consumed the
    // natural terminal event, so exact Close also waits for that same rollback.
    let _deadline = fixture
        .executor
        .block_on(fixture.owner.check_readiness_deadline(
            &fixture.captured.id,
            fixture.captured.incarnation,
            ContinueWorkReadinessWatch::new(fixture.pending.operation_id.clone()),
        ));
    fixture
        .executor
        .block_on(fixture.owner.command(OwnerCommand::Close {
            id: fixture.captured.id.clone(),
            incarnation: fixture.captured.incarnation,
        }))
        .expect("exact dead candidate rollback and physical drain ACK");
    assert_owner_fresh_launch_rolled_back(&fixture);
    fixture.shutdown();
}

/// Issue #3475: end-to-end on the runtime — a Continue work candidate whose
/// agent pane is still alive survives the base deadline instead of losing its
/// prepared successor, which is the exact regression a 12.5 MB Codex resume hit.
#[test]
fn continue_work_ready_timeout_extends_while_the_agent_pty_is_still_live() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "readiness-live-pane",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    fixture
        .executor
        .block_on(fixture.owner.check_readiness_deadline(
            &fixture.captured.id,
            fixture.captured.incarnation,
            ContinueWorkReadinessWatch::new(fixture.pending.operation_id.clone()),
        ))
        .expect("actual deadline extends live candidate");
    let live = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .expect("same real pane survives the extended deadline");
    assert!(Arc::ptr_eq(
        &live.pty,
        fixture
            .captured
            .pty
            .as_ref()
            .expect("captured actual child")
    ));
    assert!(live.has_capability_bearer(
        fixture
            .captured
            .grant
            .as_ref()
            .expect("actual Prepared grant")
    ));
    assert!(fixture
        .captured
        .issuer
        .grant_is_current(fixture.captured.grant.as_ref().unwrap()));
    assert!(gwt_core::paths::gwt_sessions_dir()
        .join(format!("{}.toml", fixture.pending.binding.session_id))
        .exists());
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .expect("binding after extension"),
        Some(fixture.predecessor_binding.clone()),
        "extending cannot activate the predecessor"
    );
    fixture.shutdown();
}

/// Issue #3482 AC-1: liveness is not identity. The deadline classifies the pane
/// it is about to act on by window presence, the Session bound to that window,
/// and the PTY process — in that order.
#[test]
fn readiness_pane_evidence_separates_live_dead_and_foreign_panes() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "readiness-evidence",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    use gwt::project_runtime::events::OwnerCommand;
    let live = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert_eq!(
        live.active_session.as_ref().unwrap().session_id,
        fixture.pending.binding.session_id
    );
    assert!(live.pty.try_wait().unwrap().is_none());
    assert!(
        fixture
            .executor
            .block_on(
                fixture
                    .owner
                    .probe(&fixture.captured.id, fixture.captured.incarnation + 1)
            )
            .is_err(),
        "a reused address with another incarnation is foreign evidence"
    );
    assert!(fixture
        .executor
        .block_on(fixture.owner.probe(
            &fixture.owner.pane_id("never-registered"),
            fixture.captured.incarnation
        ))
        .is_err());
    assert_ne!(
        live.active_session.as_ref().unwrap().session_id,
        "another-session"
    );
    fixture
        .executor
        .block_on(fixture.owner.command(OwnerCommand::Close {
            id: fixture.captured.id.clone(),
            incarnation: fixture.captured.incarnation,
        }))
        .expect("actual exact close");
    assert!(live.pty.tree_exit_confirmed());
    assert!(
        fixture
            .executor
            .block_on(
                fixture
                    .owner
                    .probe(&fixture.captured.id, fixture.captured.incarnation)
            )
            .is_err(),
        "only actual close/reap turns this exact runtime into absent evidence"
    );
    fixture.shutdown();
}

/// Issue #5194 AC-2: an unready handoff is not silent. It lands in the error
/// ledger with the hook configuration the agent should have discovered, so a
/// missing `.codex/hooks.json` is visible from `errors.list`.
#[test]
fn continue_work_ready_timeout_handoff_records_the_missing_hook_config() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (_runtime, repo, authority, work_id) =
        owner_continuation_seed(temp.path(), "handoff-hook-config");
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        use gwt::project_runtime::events::OwnerCommand;
        let owner = production_owner_fixture(&repo).await;
        let checkpoint = owner.pause_next_agent_launch().await.unwrap();
        let client = owner.client();
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::ContinueWork {
                    client_id: "test-client".into(),
                    operation_id: "handoff-hook-config".into(),
                    work_id,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual canonical ContinueWork preparation checkpoint
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        let pending = captured.pending_continue_work.as_ref().unwrap();
        checkpoint.release();
        launch.await.unwrap().unwrap();
        let session = captured.session.as_ref().unwrap();
        assert_eq!(session.agent_id, gwt_agent::AgentId::Codex);
        let hooks = session.worktree_path.join(".codex/hooks.json");
        if hooks.exists() {
            fs::remove_file(&hooks).unwrap();
        }
        let works_lock = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo)
            .with_extension("lock");
        fs::create_dir_all(works_lock.parent().unwrap()).unwrap();
        let holder = gwt_core::operation_deadline::NamedFileLock::acquire_quiet(
            &works_lock,
            "startup intake",
        )
        .unwrap();
        let probe = mature_owner_readiness_handoff(
            &owner,
            &captured.id,
            captured.incarnation,
            &pending.operation_id,
        );
        let rows = gwt_core::error_ledger::list_since(None).expect("actual error ledger");
        let row = rows
            .iter()
            .find(|row| row.target.window_id.as_deref() == Some(captured.id.as_str()))
            .unwrap();
        assert_eq!(row.kind, gwt_core::error_ledger::ErrorKind::LaunchFailure);
        assert_eq!(row.target.issue, Some(authority.number));
        assert_eq!(probe.detail.as_deref(), Some(row.message.as_str()));
        assert!(row.message.contains("SessionStart"));
        assert!(
            row.message.contains("works.lock contention observed: yes;"),
            "{}",
            row.message
        );
        assert!(
            row.message.contains(".codex") && row.message.contains("hooks.json missing"),
            "{}",
            row.message
        );
        assert!(probe.readiness_watch.as_ref().unwrap().handed_off);
        drop(holder);
        owner
            .command(OwnerCommand::Close {
                id: captured.id,
                incarnation: captured.incarnation,
            })
            .await
            .unwrap();
        owner.shutdown().await;
    });
}

/// Issue #3482 AC-2: the readiness deadline bounds *waiting*, not the life of
/// the agent. When the budget runs out on a pane that is still the exact live
/// launch pane, the launch is handed to the user with its process, its window,
/// and its prepared candidate intact.
#[test]
fn continue_work_ready_timeout_hands_off_a_live_pane_instead_of_killing_it() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "readiness-handoff-live-pane",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let live = mature_owner_readiness_handoff(
        &fixture.owner,
        &fixture.captured.id,
        fixture.captured.incarnation,
        &fixture.pending.operation_id,
    );
    assert!(live.readiness_watch.as_ref().unwrap().handed_off);
    assert!(Arc::ptr_eq(
        &live.pty,
        fixture.captured.pty.as_ref().unwrap()
    ));
    assert!(live.pty.try_wait().unwrap().is_none());
    assert!(durable_launch_recovery_exists(
        &gwt_core::paths::gwt_sessions_dir(),
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
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.predecessor_binding.clone())
    );
    fixture.shutdown();
}

/// Issue #3482 AC-4 (late ready): the whole point of keeping the pane is that a
/// slow resume can still finish. A SessionStart that lands after the handoff
/// activates the candidate exactly as an on-time one would.
#[test]
fn continue_work_ready_timeout_late_session_start_still_activates_after_a_handoff() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "readiness-late-session-start",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    let before = mature_owner_readiness_handoff(
        &fixture.owner,
        &fixture.captured.id,
        fixture.captured.incarnation,
        &fixture.pending.operation_id,
    );
    assert!(before.detail.is_some());
    assert!(before.readiness_watch.unwrap().handed_off);
    commit_owner_fresh_readiness(&fixture);
    let active = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(active.detail.is_none());
    assert!(
        active.readiness_watch.is_none(),
        "actual authenticated readiness retires its retained handoff"
    );
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

/// Issue #3482 AC-3 (identity mismatch): a readiness deadline that fires for a
/// window another Session now owns still rolls its own candidate back, but it
/// must not stop or close the pane that is there now.
#[test]
fn continue_work_ready_timeout_does_not_terminate_a_window_owned_by_another_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "readiness-foreign-window",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    use gwt::project_runtime::events::OwnerCommand;
    fixture
        .executor
        .block_on(fixture.owner.command(OwnerCommand::Close {
            id: fixture.captured.id.clone(),
            incarnation: fixture.captured.incarnation,
        }))
        .expect("retire actual predecessor and its candidate");
    assert_owner_fresh_launch_rolled_back(&fixture);
    let raw = fixture.captured.id.split_once("::").unwrap().1;
    let receipt = fixture
        .executor
        .block_on(fixture.owner.command(owner_fixture_agent_command(
            &fixture.owner,
            &fixture.repo,
            raw,
            "actual-foreign-successor",
        )))
        .unwrap();
    let incarnation = receipt["incarnation"].as_u64().unwrap();
    let successor = fixture
        .executor
        .block_on(fixture.owner.probe(&fixture.captured.id, incarnation))
        .unwrap();
    assert_ne!(
        successor.session.as_ref().unwrap().id,
        fixture.pending.binding.session_id
    );
    assert_ne!(incarnation, fixture.captured.incarnation);
    assert!(fixture
        .executor
        .block_on(fixture.owner.check_readiness_deadline(
            &fixture.captured.id,
            fixture.captured.incarnation,
            ContinueWorkReadinessWatch::new(fixture.pending.operation_id.clone())
        ))
        .is_err());
    let retained = fixture
        .executor
        .block_on(fixture.owner.probe(&fixture.captured.id, incarnation))
        .unwrap();
    assert!(Arc::ptr_eq(&retained.pty, &successor.pty));
    assert!(retained.pty.try_wait().unwrap().is_none());
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.predecessor_binding.clone())
    );
    fixture.shutdown();
}

#[test]
fn continue_work_ready_timeout_starts_only_after_pty_handoff() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (_runtime, repo, _, work_id) =
        owner_continuation_seed(temp.path(), "readiness-after-physical-install");
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        use gwt::project_runtime::{events::OwnerCommand, test_support::OwnerLaunchStage};
        let owner = production_owner_fixture(&repo).await;
        let checkpoint = owner
            .pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall)
            .await
            .unwrap();
        let client = owner.client();
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::ContinueWork {
                    client_id: "client".into(),
                    operation_id: "readiness-after-physical-install".into(),
                    work_id,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual pre-PTY materialization checkpoint; it establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        assert!(captured.pty.is_none());
        assert!(captured.session.is_some());
        assert!(owner
            .probe(&captured.id, captured.incarnation)
            .await
            .is_err());
        assert!(
            owner
                .check_readiness_deadline(
                    &captured.id,
                    captured.incarnation,
                    ContinueWorkReadinessWatch::new(captured.operation_id.clone())
                )
                .await
                .is_err(),
            "preparation cannot arm or consume a physical readiness budget"
        );
        checkpoint.release();
        launch.await.unwrap().unwrap();
        let installed = owner
            .probe(&captured.id, captured.incarnation)
            .await
            .unwrap();
        assert!(installed.pty.try_wait().unwrap().is_none());
        assert_eq!(
            installed.readiness_watch.as_ref().unwrap().operation_id,
            captured.operation_id
        );
        owner
            .command(OwnerCommand::Close {
                id: captured.id,
                incarnation: captured.incarnation,
            })
            .await
            .unwrap();
        owner.shutdown().await;
    });
}

#[test]
fn continue_work_rejects_parallel_operation_for_same_work_before_preparing_authority() {
    use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (_projection, repo, authority, work_id) =
        owner_continuation_seed(temp.path(), "parallel-owner-operation");
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        let owner = production_owner_fixture(&repo).await;
        let checkpoint = owner.pause_next_agent_launch_at(gwt::project_runtime::test_support::OwnerLaunchStage::MaterializedBeforeInstall).await.unwrap();
        let mut observed = owner.subscribe(); let client = owner.client(); let first_work = work_id.clone();
        let first = tokio::spawn(async move { client.command(OwnerCommand::ContinueWork {
            client_id: "client-1".into(), operation_id: "first-operation".into(), work_id: first_work,
        }).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual first ContinueWork worker before a competing request
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait()).await.unwrap();
        let pending = captured.pending_continue_work.as_ref().unwrap();
        let before = gwt::cli::execution_state::continuation_attempt_for_operation(&repo, authority, &pending.operation_id).unwrap().unwrap();
        let source_path = gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", pending.binding.session_id));
        let source = fs::read(&source_path).unwrap();
        owner.command(OwnerCommand::ContinueWork { client_id: "client-2".into(), operation_id: "second-operation".into(), work_id: work_id.clone() }).await.unwrap();
        let replies = std::iter::from_fn(|| observed.try_recv().ok()).collect::<Vec<_>>();
        assert!(replies.iter().any(|event| matches!(event, OwnerEvent::ContinueWorkOutcome {
            client_id, operation_id, work_id: emitted_work, outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code), retryable: true, ..
        } if client_id == "client-2" && operation_id == "second-operation" && emitted_work == &work_id && code == "continue_work_in_progress")), "{replies:?}");
        assert_eq!(owner.command(OwnerCommand::ListPanes).await.unwrap()["pending"].as_array().unwrap().len(), 1,
            "the actual first operation remains the sole pending physical owner");
        assert_eq!(gwt::cli::execution_state::continuation_attempt_for_operation(&repo, authority, &pending.operation_id).unwrap().unwrap().request, before.request);
        assert_eq!(fs::read(&source_path).unwrap(), source);
        checkpoint.release(); first.await.unwrap().unwrap();
        owner.command(OwnerCommand::Close { id: captured.id, incarnation: captured.incarnation }).await.unwrap();
        owner.shutdown().await;
    });
}

#[test]
fn continue_work_empty_operation_identity_failure_is_not_cached() {
    use gwt::project_runtime::{
        continuation::{
            coordinate_continue_work, ContinueWorkAction, ContinueWorkInput, ContinueWorkView,
        },
        events::{OwnerCommand, OwnerEvent},
    };
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("empty-operation");
    pane_control_tests::control_init_repo(&repo);
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let action = coordinate_continue_work(
        ContinueWorkInput {
            project_root: &repo,
            sessions_dir: &sessions,
            profile_config_path: None,
            is_git: true,
            migration_pending: false,
            client_id: "client-invalid",
            operation_id: "",
            work_id: "work-a",
        },
        ContinueWorkView {
            panes: &[],
            pending: &[],
            outcomes: &HashMap::new(),
            issuer: None,
        },
    );
    assert!(
        matches!(action, ContinueWorkAction::Failure { failure, cacheable: false }
        if failure.code == "invalid_request" && !failure.retryable),
        "an empty identity cannot be an owner replay-cache key"
    );
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        let owner = production_owner_fixture(&repo).await; let mut observed = owner.subscribe();
        for work_id in ["work-a", "work-b"] {
            owner.command(OwnerCommand::ContinueWork { client_id: "client-invalid".into(), operation_id: String::new(), work_id: work_id.into() }).await.unwrap();
            let replies = std::iter::from_fn(|| observed.try_recv().ok()).collect::<Vec<_>>();
            assert!(replies.iter().any(|event| matches!(event, OwnerEvent::ContinueWorkOutcome {
                client_id, operation_id, work_id: emitted_work, outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(code), retryable: false, ..
            } if client_id == "client-invalid" && operation_id.is_empty() && emitted_work == work_id && code == "invalid_request")), "{replies:?}");
        }
        assert!(owner.command(OwnerCommand::ListPanes).await.unwrap()["pending"].as_array().unwrap().is_empty());
        owner.shutdown().await;
    });
}

#[test]
fn continue_work_invalid_identity_cannot_overwrite_cached_outcome_or_drain_waiters() {
    use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (_projection, repo, _authority, work_id) =
        owner_continuation_seed(temp.path(), "invalid-owner-operation");
    let codex_home = temp.path().join("codex-home");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    persist_owner_continue_conversation(&repo, &codex_home, "historical-session", &repo);
    let operation_id = "immutable-operation";
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        let owner = production_owner_fixture(&repo).await; let mut observed = owner.subscribe();
        let checkpoint = owner.pause_next_agent_launch().await.unwrap(); let client = owner.client(); let initial_work = work_id.clone();
        let initial = tokio::spawn(async move { client.command(OwnerCommand::ContinueWork {
            client_id: "client-initial".into(), operation_id: operation_id.into(), work_id: initial_work,
        }).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual ContinueWork before duplicate waiter and invalid identity requests
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait()).await.unwrap();
        let pending = captured.pending_continue_work.as_ref().unwrap();
        assert_eq!(pending.outcome, gwt::ContinueWorkOutcomeKind::ContinuedConversation);
        assert_eq!(owner.command(OwnerCommand::ContinueWork { client_id: "client-waiter".into(), operation_id: operation_id.into(), work_id: work_id.clone() }).await.unwrap()["pending"], true);
        owner.command(OwnerCommand::ContinueWork { client_id: "client-invalid".into(), operation_id: operation_id.into(), work_id: String::new() }).await.unwrap();
        let invalid = std::iter::from_fn(|| observed.try_recv().ok()).filter(|event| matches!(event, OwnerEvent::ContinueWorkOutcome { .. })).collect::<Vec<_>>();
        assert_eq!(invalid.len(), 1, "invalid requests cannot drain real pending waiters: {invalid:?}");
        assert!(matches!(&invalid[0], OwnerEvent::ContinueWorkOutcome { client_id, operation_id: emitted_operation, work_id, outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code), retryable: false, .. } if client_id == "client-invalid" && emitted_operation == operation_id && work_id.is_empty() && code == "invalid_request"));
        checkpoint.release(); initial.await.unwrap().unwrap();
        let live = owner.probe(&captured.id, captured.incarnation).await.unwrap();
        let mut readiness = runtime_hook_state_for_event("Waiting", "SessionStart", &pending.binding.session_id);
        readiness.project_root = Some(repo.display().to_string());
        readiness.branch = Some(pending.work_branch.clone());
        readiness.continuation_readiness_nonce = Some(pending.readiness_nonce.clone());
        assert_eq!(live.runtime_hook(&readiness).await.unwrap(), reqwest::StatusCode::NO_CONTENT);
        let mut completed = Vec::new();
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual authenticated ContinueWork readiness worker and its correlated recipients
        tokio::time::timeout(Duration::from_secs(30), async {
            while completed.len() < 2 {
                if let OwnerEvent::ContinueWorkOutcome { client_id, operation_id: emitted_operation, work_id: emitted_work, outcome,
                    error_code, retryable, .. } = observed.recv().await.unwrap() {
                    assert_eq!(emitted_operation, operation_id);
                    assert_eq!(emitted_work, work_id);
                    assert_eq!(outcome, gwt::ContinueWorkOutcomeKind::ContinuedConversation);
                    assert!(error_code.is_none()); assert!(!retryable);
                    completed.push(client_id);
                }
            }
        }).await.unwrap();
        assert_eq!(completed.len(), 2, "actual completion must retain each original recipient: {completed:?}");
        for client in ["client-initial", "client-waiter"] { assert_eq!(completed.iter().filter(|id| id.as_str() == client).count(), 1); }
        owner.command(OwnerCommand::ContinueWork { client_id: "client-invalid".into(), operation_id: operation_id.into(), work_id: String::new() }).await.unwrap();
        let _invalid_after_completion = std::iter::from_fn(|| observed.try_recv().ok()).collect::<Vec<_>>();
        owner.command(OwnerCommand::ContinueWork { client_id: "client-replay".into(), operation_id: operation_id.into(), work_id: work_id.clone() }).await.unwrap();
        let replay = std::iter::from_fn(|| observed.try_recv().ok()).filter(|event| matches!(event, OwnerEvent::ContinueWorkOutcome { .. })).collect::<Vec<_>>();
        assert_eq!(replay.len(), 1, "successful cached replay must not redrain completed waiters: {replay:?}");
        assert!(matches!(&replay[0], OwnerEvent::ContinueWorkOutcome { client_id, operation_id: emitted_operation, work_id: emitted_work, outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            error_code: None, retryable: false, .. } if client_id == "client-replay" && emitted_operation == operation_id && emitted_work == &work_id));
        owner.command(OwnerCommand::ContinueWork { client_id: "client-conflict".into(), operation_id: operation_id.into(), work_id: "work-b".into() }).await.unwrap();
        let conflict = std::iter::from_fn(|| observed.try_recv().ok()).collect::<Vec<_>>();
        assert!(conflict.iter().any(|event| matches!(event, OwnerEvent::ContinueWorkOutcome { client_id, operation_id: emitted_operation, work_id, outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code), retryable: false, .. } if client_id == "client-conflict" && emitted_operation == operation_id && work_id == "work-b" && code == "operation_conflict")), "{conflict:?}");
        owner.command(OwnerCommand::Close { id: captured.id, incarnation: captured.incarnation }).await.unwrap();
        owner.shutdown().await;
    });
}

use super::*;

#[test]
fn app_runtime_issue_launch_wizard_seeds_issue_workspace_context() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3096,
            "Fix Launch Agent trace",
            &["bug"],
            "Launch trace missing",
            "2026-06-20T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_knowledge_launch_wizard_for_base_branch(
            "tab-1",
            &repo,
            "develop",
            3096,
            LinkedIssueKind::Issue,
        )
        .expect("open issue launch wizard");

    let context = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.workspace_resume_context.as_ref())
        .expect("issue launch wizard should carry workspace context");
    assert_eq!(context.owner.as_deref(), Some("Issue #3096"));
    assert_eq!(context.title.as_deref(), Some("Fix Launch Agent trace"));
}

// #3426: the unified Issue surface preset collapses to LinkedIssueKind::Issue,
// so the wizard must re-canonicalize the kind from the cached gwt-spec label
// before seeding the Work owner context.
#[test]
fn app_runtime_issue_launch_wizard_prefers_cached_spec_label_over_preset_kind() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            1921,
            "SPEC-1921: agent management",
            &["gwt-spec", "phase/implementation"],
            "spec body",
            "2026-08-03T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_knowledge_launch_wizard_for_base_branch(
            "tab-1",
            &repo,
            "develop",
            1921,
            LinkedIssueKind::Issue,
        )
        .expect("open issue launch wizard");

    let context = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.workspace_resume_context.as_ref())
        .expect("issue launch wizard should carry workspace context");
    assert_eq!(
        context.owner.as_deref(),
        Some("SPEC-1921"),
        "cached gwt-spec label evidence must override the collapsed preset kind,          in the durable binding spelling (SPEC #3431 FR-070)"
    );
    // #3426: the re-canonicalization is scoped to the owner label. The wizard's
    // own linked-issue kind still comes from the caller, because it also drives
    // the read-only "Linked issue" section and the manual branch suffix, and a
    // gwt-spec label must not hide the section or seed `spec-1921`.
    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .wizard
        .view();
    assert!(
        view.show_linked_issue,
        "a cached gwt-spec Issue opened from the unified surface must keep its Linked issue section"
    );
    assert_eq!(view.branch_name, "work/issue-1921");
}

/// SPEC #3431 FR-070: a spec-linked launch must seed the same owner spelling
/// the durable execution binding produces (`SPEC-<n>`). The wizard used to
/// write `SPEC #<n>`, which no resolver ever emits, so `workspace.ensure`
/// rejected every spec-launched agent as an owner mismatch and left it unable
/// to set its own title-summary for the whole life of the Work.
#[test]
fn app_runtime_spec_launch_wizard_seeds_canonical_spec_workspace_owner() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3431,
            "PM エージェント",
            &["gwt-spec"],
            "Resident project manager",
            "2026-08-07T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_knowledge_launch_wizard_for_base_branch(
            "tab-1",
            &repo,
            "develop",
            3431,
            LinkedIssueKind::Spec,
        )
        .expect("open spec launch wizard");

    let context = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.workspace_resume_context.as_ref())
        .expect("spec launch wizard should carry workspace context");
    assert_eq!(
        context.owner.as_deref(),
        Some("SPEC-3431"),
        "the wizard owner label must match the durable binding spelling, or          workspace.ensure fails with an unrecoverable owner mismatch"
    );
}

#[test]
fn app_runtime_issue_launch_completion_records_issue_owned_start_work_event() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3096");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    let mut active = sample_active_agent_session("tab-1", &combined_window_id("tab-1", "agent-1"));
    active.session_id = "session-issue-3096".into();
    active.branch_name = "work/issue-3096".into();
    active.worktree_path = worktree.clone();
    active.agent_project_root = repo.display().to_string();
    let context = WorkspaceResumeContext {
        title: Some("Fix Launch Agent trace".into()),
        owner: Some("Issue #3096".into()),
        summary: None,
        next_action: None,
    };
    gwt::project_runtime::workspace::save_start_work_workspace_projection(
        &repo,
        &active,
        "develop",
        Some(3096),
        None,
        Some(&context),
        None,
    )
    .expect("owner publishes issue-owned Start Work");

    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load work items")
        .expect("work items");
    let item = work_items
        .work_items
        .iter()
        .find(|item| item.owner.as_deref() == Some("Issue #3096"))
        .expect("issue-owned work item");
    assert_eq!(item.title, "Fix Launch Agent trace");
    assert_eq!(item.agents[0].session_id, "session-issue-3096");
    assert_eq!(
        item.events[0].kind,
        gwt_core::workspace_projection::WorkEventKind::Start
    );
    assert_eq!(
        item.execution_containers[0].branch.as_deref(),
        Some("work/issue-3096")
    );
}

#[test]
fn resume_completion_retires_only_a_replaced_source_session() {
    for reuse_source in [true, false] {
        let temp = tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(temp.path());
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut resumed =
            gwt_agent::Session::new(temp.path(), "feature/test", gwt_agent::AgentId::Codex);
        resumed.update_status(gwt_agent::AgentStatus::Running);
        resumed.restore_window_on_startup = true;
        resumed
            .save(&sessions_dir)
            .expect("completed launch Session");
        let session_path = sessions_dir.join(format!("{}.toml", resumed.id));
        let mut source = resumed.clone();
        if !reuse_source {
            source.id = "replaced-source".into();
        }
        source.save(&sessions_dir).expect("source Session");
        let source_path = sessions_dir.join(format!("{}.toml", source.id));
        gwt::project_runtime::installation::retire_restore_source_after_launch(
            &sessions_dir,
            &source.id,
            Some(&source),
            &resumed.id,
        )
        .expect("same production owner source retirement boundary");
        let resumed = gwt_agent::Session::load(&session_path).expect("resumed Session");
        assert_eq!(resumed.status, gwt_agent::AgentStatus::Running);
        assert!(resumed.restore_window_on_startup);
        let source = gwt_agent::Session::load(&source_path).expect("source after resume");
        assert_eq!(source.restore_window_on_startup, reuse_source);
        assert_eq!(
            source.status,
            if reuse_source {
                gwt_agent::AgentStatus::Running
            } else {
                gwt_agent::AgentStatus::Stopped
            }
        );
    }
}

#[test]
fn launch_complete_registers_monitor_runtime_from_worker_snapshot() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    init_repo_with_initial_commit(temp.path());
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let mut named = gwt_agent::Session::new(
        temp.path(),
        "main",
        gwt_agent::AgentId::Custom("fixture".into()),
    );
    named.agent_name = Some("ミオ".into());
    named.linked_issue_number = Some(5187);
    named.save(&sessions_dir).expect("named member seed");
    gwt::agent_names::adopt_project_member_session(temp.path(), &sessions_dir, &mut named)
        .expect("existing persistent named member");
    let mut predecessor = named.clone();
    predecessor.id = "previous-named-session".into();
    predecessor.agent_name = Some("ハヤテ".into());
    predecessor.agent_member_id = None;
    predecessor.save(&sessions_dir).expect("exact predecessor");
    let client = Arc::new(FakeIssueClient::new());
    let mut issue = sample_issue_snapshot(5187, "named work", &[], "", "2026-10-15T00:00:00Z");
    issue.comments.clear();
    client.seed(issue);
    let gui_thread = std::thread::current().id();
    let recording_client = client.clone();
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::OwnerCommand;
        let owner = production_owner_fixture(temp.path()).await;
        owner
            .set_issue_client_factory(Arc::new(move |_, _| {
                assert_ne!(
                    std::thread::current().id(),
                    gui_thread,
                    "Issue history must run off the GUI thread"
                );
                Ok(recording_client.clone() as Arc<dyn IssueClient>)
            }))
            .await
            .expect("isolated actual owner Issue client");
        let checkpoint = owner
            .pause_next_agent_launch()
            .await
            .expect("actual prepared worker snapshot");
        let OwnerCommand::LaunchAgent {
            id,
            operation_id,
            tab_id,
            mut config,
            metadata,
            cols,
            rows,
        } = owner_fixture_agent_command(
            &owner,
            temp.path(),
            "captured-monitor",
            "captured-monitor-operation",
        )
        else {
            unreachable!("agent fixture")
        };
        config.launch_route = gwt_agent::LaunchRoute::Autonomous;
        config.linked_issue_number = Some(5187);
        config.suppress_execution_control = true;
        config.predecessor_session_id = Some(predecessor.id.clone());
        let command = OwnerCommand::LaunchAgent {
            id: id.clone(),
            operation_id,
            tab_id,
            config,
            metadata,
            cols,
            rows,
        };
        let transport = owner.client();
        let launch = tokio::spawn(async move { transport.command(command).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .expect("actual preparation reaches the checkpoint");
        let snapshot = captured.session.as_ref().expect("actual prepared Session");
        assert_eq!(snapshot.agent_name.as_deref(), Some("ミオ"));
        let session_path = sessions_dir.join(format!("{}.toml", snapshot.id));
        fs::write(&session_path, "invalid Session TOML")
            .expect("make Session unreadable after preparation");
        assert!(
            client.comments(IssueNumber(5187)).is_empty(),
            "preparation must not publish Issue history"
        );
        checkpoint.release();
        launch
            .await
            .expect("launch request")
            .expect("actual owner installation");
        let physical = owner
            .probe(&id, captured.incarnation)
            .await
            .expect("actual owner physical runtime");
        let writer = physical.writer.as_ref().expect("owner PTY writer");
        let registration = writer
            .monitor_runtime
            .as_ref()
            .expect("registration must use the prepared Session snapshot");
        snapshot
            .save(&sessions_dir)
            .expect("restore only fixture Session for exact cleanup");
        owner
            .command(OwnerCommand::Close {
                id,
                incarnation: captured.incarnation,
            })
            .await
            .expect("exact cleanup");
        owner.shutdown().await;
        assert_eq!(registration.session_id, snapshot.id);
        assert_eq!(registration.issue_number, 5187);
        assert_eq!(
            registration.project_root,
            temp.path().canonicalize().unwrap()
        );
        assert_eq!(registration.incarnation, captured.incarnation);
        assert!(Arc::ptr_eq(&writer.handle, &physical.pty));
    });
    assert!(
        client
            .comments(IssueNumber(5187))
            .iter()
            .any(|comment| comment.body.contains("ハヤテ から ミオ に引き継ぎ")),
        "successful launch must record the prepared name and exact predecessor"
    );
}

#[test]
fn launch_complete_worker_finishes_when_session_metadata_lock_is_busy() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::OwnerCommand;
        let owner = production_owner_fixture(temp.path()).await;
        let checkpoint = owner
            .pause_next_agent_launch()
            .await
            .expect("capture the real prepared worker result before owner installation");
        let command = owner_fixture_agent_command(
            &owner,
            temp.path(),
            "busy-metadata",
            "busy-metadata-operation",
        );
        let client = owner.client();
        let mut launch = tokio::spawn(async move { client.command(command).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .expect("actual preparation reaches the checkpoint");
        let session = captured.session.as_ref().expect("materialized Session");
        assert!(
            captured
                .pty
                .as_ref()
                .is_some_and(|pty| pty.try_wait().unwrap().is_none()),
            "the completed worker must retain its exact live PTY before mailbox installation"
        );
        let session_lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(gwt_core::paths::gwt_sessions_dir().join(format!(".{}.lock", session.id)))
            .expect("Session sidecar lock");
        FileExt::lock_exclusive(&session_lock).expect("hold Session metadata lock");
        checkpoint.release();
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual launch ACK while the captured Session lease remains held
        let completed = tokio::time::timeout(Duration::from_secs(10), &mut launch).await;
        FileExt::unlock(&session_lock).expect("release Session metadata lock");
        let receipt = match completed {
            Ok(result) => result
                .expect("actual owner preparation worker")
                .expect("owner launch completion"),
            Err(error) => {
                let _ = launch.await;
                owner.shutdown().await;
                panic!("Session lock contention must not pin launch preparation: {error}");
            }
        };
        let incarnation = receipt["incarnation"].as_u64().expect("actual incarnation");
        owner
            .probe(&captured.id, incarnation)
            .await
            .expect("owner installed physical runtime");
        owner
            .command(OwnerCommand::Close {
                id: captured.id,
                incarnation,
            })
            .await
            .expect("exact cleanup");
        owner.shutdown().await;
    });
}

#[test]
fn launch_complete_replays_session_start_received_before_pane_install() {
    assert_owner_early_hook_replay(false);
}

#[test]
fn launch_complete_replayed_session_start_precedes_later_queued_hook() {
    assert_owner_early_hook_replay(true);
}

fn assert_owner_early_hook_replay(later_hook: bool) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::{
            events::{OwnerCommand, OwnerEvent},
            test_support::OwnerLaunchStage,
        };
        let owner = production_owner_fixture(temp.path()).await;
        let mut events = owner.subscribe();
        let checkpoint = owner
            .pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall)
            .await
            .expect("actual owner preparation checkpoint");
        let command =
            owner_fixture_agent_command(&owner, temp.path(), "early-hook", "early-hook-operation");
        let client = owner.client();
        let launch = tokio::spawn(async move { client.command(command).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .expect("actual preparation reaches the checkpoint");
        let session = captured
            .session
            .as_ref()
            .expect("actual materialized Session");
        let grant = captured.grant.as_ref().expect("the same actual Host grant");
        assert!(captured.issuer.grant_is_current(grant));
        assert_eq!(grant.principal().session_id(), session.id);
        let mut early = runtime_hook_state_for_event("Waiting", "SessionStart", &session.id);
        early.project_root = Some(temp.path().display().to_string());
        early.branch = Some(session.branch.clone());
        owner.transport_event(gwt::project_transport::TransportEvent::RuntimeHook(early));
        owner
            .command(OwnerCommand::Health)
            .await
            .expect("owner queued the early hook");
        let pending = owner
            .command(OwnerCommand::ListPanes)
            .await
            .expect("pending owner inventory");
        assert!(pending["panes"].as_array().unwrap().is_empty());
        assert_eq!(pending["pending"].as_array().unwrap().len(), 1);
        checkpoint.release();
        let receipt = launch
            .await
            .expect("launch worker")
            .expect("actual launch ACK");
        let incarnation = receipt["incarnation"].as_u64().expect("exact incarnation");
        if later_hook {
            let mut later = runtime_hook_state_for_event("Running", "PreToolUse", &session.id);
            later.project_root = Some(temp.path().display().to_string());
            later.branch = Some(session.branch.clone());
            owner.transport_event(gwt::project_transport::TransportEvent::RuntimeHook(later));
        }
        owner
            .command(OwnerCommand::Health)
            .await
            .expect("hook replay barrier");
        let snapshot = owner
            .command(OwnerCommand::Snapshot {
                id: captured.id.clone(),
                incarnation,
            })
            .await
            .expect("owner snapshot after replay");
        assert_eq!(snapshot["session_id"], session.id);
        assert_eq!(
            snapshot["status"],
            if later_hook { "running" } else { "idle" }
        );
        if later_hook {
            let statuses = std::iter::from_fn(|| events.try_recv().ok())
                .filter_map(|event| match event {
                    OwnerEvent::Status { id, status, .. } if id == captured.id => Some(status),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                statuses.windows(2).any(|statuses| statuses
                    == [WindowProcessStatus::Idle, WindowProcessStatus::Running]),
                "early SessionStart must be applied before the later PreToolUse hook: {statuses:?}"
            );
        }
        owner
            .command(OwnerCommand::Close {
                id: captured.id,
                incarnation,
            })
            .await
            .expect("exact cleanup");
        owner.shutdown().await;
    });
}

#[test]
fn continue_work_launch_complete_canceled_before_pty_aborts_exact_candidate() {
    assert_owner_continue_cancellation(
        gwt::project_runtime::test_support::OwnerLaunchStage::MaterializedBeforeInstall,
        false,
    );
}

#[test]
fn continue_work_launch_complete_stale_after_pty_aborts_exact_candidate() {
    assert_owner_continue_cancellation(
        gwt::project_runtime::test_support::OwnerLaunchStage::PreparedBeforeMailbox,
        false,
    );
}

#[test]
fn continue_work_launch_complete_stale_preserves_activated_generation() {
    assert_owner_continue_cancellation(
        gwt::project_runtime::test_support::OwnerLaunchStage::PreparedBeforeMailbox,
        true,
    );
}

fn assert_owner_continue_cancellation(
    stage: gwt::project_runtime::test_support::OwnerLaunchStage,
    activated: bool,
) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (_runtime, repo, owner_key, work_id) =
        owner_continuation_seed(temp.path(), "owner-cancel-continuation");
    let predecessor_binding =
        gwt::cli::execution_state::current_execution_binding(&repo, owner_key)
            .expect("predecessor authority")
            .expect("predecessor binding");
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::OwnerCommand;
        let owner = production_owner_fixture(&repo).await;
        let checkpoint = owner
            .pause_next_agent_launch_at(stage)
            .await
            .expect("actual continuation checkpoint");
        let client = owner.client();
        let selected_work = work_id.clone();
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::ContinueWork {
                    client_id: "client-1".into(),
                    operation_id: "owner-cancel-operation".into(),
                    work_id: selected_work,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .expect("actual candidate prepared");
        let session = captured.session.as_ref().expect("actual candidate Session");
        let binding = session
            .execution_binding
            .clone()
            .expect("actual candidate binding");
        let attempt = gwt::cli::execution_state::continuation_attempt_for_operation(
            &repo,
            owner_key,
            &captured.operation_id,
        )
        .expect("prepared owner attempt")
        .expect("actual attempt");
        assert_eq!(attempt.request.initial_session_id, session.id);
        if activated {
            gwt::cli::execution_state::activate_successor(&repo, owner_key, &attempt.request)
                .expect("activation committed before completion loss");
        }
        let client = owner.client();
        let id = captured.id.clone();
        let operation_id = captured.operation_id.clone();
        let cancel = tokio::spawn(async move {
            client
                .command(OwnerCommand::PendingCancel { id, operation_id })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for observing the actual owner cancellation flag, not a timing assertion
        tokio::time::timeout(Duration::from_secs(30), async {
            while captured.current.load(std::sync::atomic::Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("canonical exact cancellation admitted");
        checkpoint.release();
        assert!(launch.await.expect("launch request").is_err());
        let cleanup = cancel.await.expect("cancellation request");
        cleanup.expect("owner exact cleanup ACK");
        if let Some(pty) = captured.pty.as_ref() {
            assert!(
                pty.try_wait().expect("captured child status").is_some(),
                "captured physical child is reaped"
            );
        }
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        if activated {
            assert_eq!(
                gwt::cli::execution_state::current_execution_binding(&repo, owner_key)
                    .expect("committed generation"),
                Some(binding.identity.clone())
            );
            let retained =
                gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", session.id)))
                    .expect("activated Session remains discoverable");
            assert_eq!(retained.execution_binding.as_ref(), Some(&binding));
        } else {
            assert!(
                !sessions_dir.join(format!("{}.toml", session.id)).exists(),
                "a canceled prepared continuation must not remain restorable"
            );
            assert!(!gwt_agent::runtime_state_path(&sessions_dir, &session.id).exists());
            assert_eq!(
                gwt::cli::execution_state::current_execution_binding(&repo, owner_key)
                    .expect("preserved predecessor"),
                Some(predecessor_binding)
            );
            let work = gwt_core::workspace_projection::load_workspace_work_items(&repo)
                .expect("unchanged Work")
                .expect("selected Work");
            assert!(work
                .work_items
                .iter()
                .all(|item| item.id != format!("work-session-{}", session.id)));
            assert_eq!(
                work.work_items
                    .iter()
                    .find(|item| item.id == work_id)
                    .expect("selected Work remains")
                    .status_category,
                gwt_core::workspace_projection::WorkspaceStatusCategory::Done
            );
        }
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &repo,
                owner_key,
                &captured.operation_id
            )
            .expect("settled attempt")
            .expect("attempt")
            .status,
            if activated {
                gwt::cli::execution_state::ContinuationAttemptStatus::Activated
            } else {
                gwt::cli::execution_state::ContinuationAttemptStatus::Aborted
            }
        );
        owner.shutdown().await;
    });
}

#[test]
fn launch_failure_status_preserves_worker_profile_and_uses_background_snapshot_order() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, delivery) = owner_monitor_fixture(temp.path(), 42, true);
    let mut config = super::pane_control_tests::control_launch_config(&root, 42);
    config.model = Some("worker-profile".into());
    let mut profile = gwt::IssueMonitorLaunchProfile::from(&config);
    profile.agent_id = "codex".into();
    gwt::daemon_publisher::publish_durable_issue_monitor_control(
        &root,
        gwt::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({"profiles_set":{"profiles":[profile]}}),
            std::process::id(),
        ),
    )
    .unwrap();
    let (detail, evidence, events) = owner_monitor_fail_before_pty(
        &root,
        &owner,
        42,
        "failed-profile",
        delivery,
        Some("worker-profile"),
        gwt_agent::AgentId::Codex,
    );
    assert!(events.iter().any(|event| matches!(event, gwt::project_runtime::events::OwnerEvent::WindowRemoved { id, incarnation } if id == &evidence.id && *incarnation == evidence.incarnation)));
    let mut stale_gui_session = evidence.session.unwrap();
    assert_eq!(stale_gui_session.agent_id, gwt_agent::AgentId::Codex);
    assert_eq!(stale_gui_session.model.as_deref(), Some("worker-profile"));
    stale_gui_session.model = Some("gui-profile".into());
    stale_gui_session.updated_at += chrono::Duration::seconds(1);
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-1",
            "Repo",
            root.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab-1"),
    );
    runtime
        .launch_wizard_cache
        .record_session(stale_gui_session);
    let monitor = owner_monitor_read(&root, 42);
    assert_eq!(
        monitor
            .prefs()
            .failed_issues
            .iter()
            .find(|failure| failure.issue_number == 42)
            .map(|failure| failure.message.as_str()),
        Some(detail.as_str())
    );
    let snapshots = gwt::issue_monitor_worker::issue_monitor_read_only_daemon_payloads(&monitor);
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].event, "status");
    assert_eq!(snapshots[1].event, "inbox");
    let summary = snapshots[0].payload["launch_profile_summary"]
        .as_str()
        .unwrap();
    assert!(summary.contains("worker-profile"));
    assert!(!summary.contains("gui-profile"));
}

#[test]
fn launch_failure_dispatch_uses_worker_receipt_when_owner_ledger_is_unreadable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, repo, owner_key, work_id) =
        owner_continuation_seed(temp.path(), "owner-worker-failure-receipt");
    let predecessor = gwt::cli::execution_state::current_execution_binding(&repo, owner_key)
        .expect("predecessor");
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
        let owner = production_owner_fixture(&repo).await;
        owner.set_pty_start_gate(temp.path().join("missing-pty-gate"), Vec::new()).await.expect("actual PTY spawn failure");
        let checkpoint = owner.pause_next_agent_launch().await.expect("post-compensation mailbox checkpoint");
        let mut projected = owner.subscribe();
        let transport = owner.client();
        let selected_work = work_id.clone();
        let launch = tokio::spawn(async move { transport.command(OwnerCommand::ContinueWork {
            client_id: "client-1".into(), operation_id: "owner-failure-receipt".into(), work_id: selected_work,
        }).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait()).await.expect("actual preparation reaches the checkpoint");
        let attempt = gwt::cli::execution_state::continuation_attempt_for_operation(&repo, owner_key, &captured.operation_id)
            .expect("compensated attempt").expect("attempt");
        assert_eq!(attempt.status, gwt::cli::execution_state::ContinuationAttemptStatus::Aborted);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        assert!(!sessions_dir.join(format!("{}.toml", attempt.request.initial_session_id)).exists());
        assert!(!gwt_agent::runtime_state_path(&sessions_dir, &attempt.request.initial_session_id).exists());
        assert_eq!(gwt::cli::execution_state::current_execution_binding(&repo, owner_key).expect("preserved predecessor"), predecessor);
        let unchanged = gwt_core::workspace_projection::load_workspace_work_items(&repo).expect("selected Work").expect("Work");
        assert_eq!(unchanged.work_items.iter().find(|work| work.id == work_id).expect("same Work").status_category,
            gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
        let artifacts = exact_continue_authority_artifacts(&repo, owner_key);
        let ledger_path = artifacts.last().expect("owner ledger");
        let original_ledger = fs::read(ledger_path).expect("capture only our faulted ledger");
        fs::write(ledger_path, b"invalid owner ledger").expect("make post-worker ledger unreadable");
        let authority_before = snapshot_optional_files(&artifacts);
        let work_before = tracked_work_event_store_snapshot(&repo);
        checkpoint.release();
        let launch_result = launch.await.expect("launch request");
        owner.command(OwnerCommand::Health).await.expect("actor apply barrier");
        let identity = owner.client().identity().cloned().expect("authenticated owner");
        let context = runtime.test_context();
        let state = runtime.project_state_mut(&context).expect("current project read model");
        state.project_owner_connection_id = 1;
        state.project_owner_identity = Some(identity.clone());
        let mut events = Vec::new();
        for event in std::iter::from_fn(|| projected.try_recv().ok()) {
            if matches!(event, OwnerEvent::ContinueWorkOutcome { .. }) {
                events.extend(runtime.handle_project_owner_event(&context, 1, &identity, event));
            }
        }
        let inventory = owner.command(OwnerCommand::ListPanes).await.expect("settled owner inventory");
        let authority_after = snapshot_optional_files(&artifacts);
        let work_after = tracked_work_event_store_snapshot(&repo);
        fs::write(ledger_path, original_ledger).expect("release only the isolated post-worker read fault");
        owner.shutdown().await;
        assert!(launch_result.is_err(), "the actual failed spawn must return its failure: {launch_result:?}");
        assert!(events.iter().any(|event| matches!(&event.event,
            BackendEvent::ContinueWorkOutcome { outcome: gwt::ContinueWorkOutcomeKind::Failed, error_code: Some(code), .. } if code == "launch_failed")),
            "GUI failure apply must use the committed worker receipt");
        assert!(runtime.pending_continue_work.is_empty());
        assert!(inventory["pending"].as_array().unwrap().is_empty());
        assert_eq!(authority_after, authority_before);
        assert_eq!(work_after, work_before);
    });
}

#[test]
fn launch_failure_dispatch_admission_rejection_retains_authority() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut fixture, _, _, _) = queued_continue_work_completion_fixture(temp.path());
    fixture.runtime.blocking_tasks = BlockingTaskSpawner::failing("worker unavailable");
    fixture.runtime.pending_launch_feedback_contexts.insert(
        fixture.window_id.clone(),
        issue_monitor_feedback(fixture.owner.number),
    );
    fixture
        .runtime
        .agent_capability_tokens
        .insert(fixture.window_id.clone(), "admission-capability".into());
    let authority_before = snapshot_optional_files(&exact_continue_authority_artifacts(
        &fixture.repo,
        fixture.owner,
    ));
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    let candidate_before = fs::read(&candidate_path).expect("candidate Session");
    let work_before = tracked_work_event_store_snapshot(&fixture.repo);
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&fixture.repo)
        .build();
    let generation = fixture
        .runtime
        .window_lifecycle_generations
        .lock()
        .unwrap()
        .get(&fixture.window_id)
        .copied();
    let error = fixture
        .runtime
        .queue_project_owner_launch(
            &fixture.runtime.test_context(),
            fixture.window_id.clone(),
            generation,
            Box::new(config),
            Box::default(),
        )
        .expect_err("unavailable GUI worker must refuse before owner admission");
    let events = fixture.runtime.status_events(
        fixture.window_id.clone(),
        WindowProcessStatus::Error,
        Some(error),
    );
    assert!(
        fixture
            .runtime
            .pending_launch_feedback_contexts
            .contains_key(&fixture.window_id),
        "admission rejection must retain launch feedback for recovery"
    );
    assert!(fixture
        .runtime
        .pending_continue_work
        .contains_key(&fixture.window_id));
    assert_eq!(
        fixture
            .runtime
            .agent_capability_tokens
            .get(&fixture.window_id)
            .map(String::as_str),
        Some("admission-capability")
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Error,
            ..
        }
    )));
    assert_eq!(
        fs::read(candidate_path).expect("retained candidate"),
        candidate_before
    );
    assert_optional_files_unchanged(&authority_before);
    assert_eq!(
        tracked_work_event_store_snapshot(&fixture.repo),
        work_before
    );
}

#[test]
fn launch_complete_manual_spawn_failure_keeps_session_for_restart() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    let owner = executor.block_on(production_owner_fixture(temp.path()));
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().into(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded) =
        sample_runtime_with_events(&temp.path().join(".gwt"), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = combined_window_id("tab-1", "agent-1");
    let custom_agent = gwt_agent::AgentId::Custom("missing-restart-fixture".into());
    executor
        .block_on(owner.set_pty_start_gate(temp.path().join("missing-pty-gate"), Vec::new()))
        .expect("select actual physical spawn failure after Session preparation");
    let gwt::project_runtime::events::OwnerCommand::LaunchAgent { mut config, .. } =
        owner_fixture_agent_command(
            &owner,
            temp.path(),
            "failed-manual",
            "failed-manual-operation",
        )
    else {
        unreachable!("agent fixture")
    };
    config.agent_id = custom_agent.clone();
    let generation = runtime
        .window_lifecycle_generations
        .lock()
        .unwrap()
        .get(&window_id)
        .copied();
    runtime
        .queue_project_owner_launch(
            &runtime.test_context(),
            window_id.clone(),
            generation,
            config,
            Box::default(),
        )
        .expect("queue immutable manual launch");
    drain_queued_blocking_tasks(&tasks);
    let completion = take_remote_owner_launch_completion(&recorded);
    assert!(
        matches!(
            &completion.result,
            Ok(super::super::RemoteOwnerLaunchOutcome::Failed { .. })
        ),
        "actual PTY spawn failure must return its retained diagnostic Session"
    );
    runtime.handle_remote_owner_launch_complete(completion);
    let retained = fs::read_dir(&runtime.sessions_dir)
        .expect("Session store")
        .flatten()
        .filter_map(|entry| gwt_agent::Session::load(&entry.path()).ok())
        .filter(|session| session.agent_id == custom_agent)
        .collect::<Vec<_>>();
    assert_eq!(
        retained.len(),
        1,
        "one exact failed manual Session remains restartable"
    );
    let session_id = retained[0].id.clone();
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Error)
    );
    let address = runtime.window_lookup[&window_id].clone();
    assert_eq!(
        runtime
            .tab(&address.tab_id)
            .unwrap()
            .workspace
            .window(&address.raw_id)
            .unwrap()
            .session_id
            .as_deref(),
        Some(session_id.as_str()),
        "a retained manual error pane needs its Session for Restart"
    );
    let events = runtime.restart_window_events(&window_id);
    assert!(!events.is_empty());
    assert!(runtime
        .pending_auto_resume_sources
        .values()
        .any(|source| source == &session_id));
    drain_queued_blocking_tasks(&tasks);
    executor.block_on(owner.shutdown());
}

#[test]
fn launch_complete_rejects_closed_window_and_defers_pty_cleanup() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _process_home = ScopedEnvVar::set("HOME", temp.path());
    let _process_userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    let owner = executor.block_on(production_owner_fixture(temp.path()));
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().into(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded) =
        sample_runtime_with_events(&temp.path().join(".gwt"), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = combined_window_id("tab-1", "agent-1");
    let context = runtime.test_context();
    let generation = runtime
        .window_lifecycle_generations
        .lock()
        .unwrap()
        .get(&window_id)
        .copied();
    let gwt::project_runtime::events::OwnerCommand::LaunchAgent {
        config, metadata, ..
    } = owner_fixture_agent_command(&owner, temp.path(), "closed-window", "closed-window-launch")
    else {
        unreachable!("agent fixture command")
    };
    runtime
        .queue_project_owner_launch(&context, window_id.clone(), generation, config, metadata)
        .expect("queue immutable launch");
    drain_queued_blocking_tasks(&tasks);
    let completion = take_remote_owner_launch_completion(&recorded);
    let (session_id, child_pid) = match &completion.result {
        Ok(super::super::RemoteOwnerLaunchOutcome::Runtime(snapshot, _)) => (
            snapshot.session_id.clone().expect("actual Session"),
            snapshot.child_pid.expect("actual child"),
        ),
        _ => panic!("owner must acknowledge the exact live physical launch"),
    };
    runtime.close_window_events(&window_id);
    assert!(runtime
        .handle_remote_owner_launch_complete(completion)
        .is_empty());
    assert!(!runtime.runtimes.contains_key(&window_id));
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(
        !tasks.lock().unwrap().is_empty(),
        "PTY cleanup must run off the GUI thread"
    );
    assert!(
        gwt::process::is_process_alive(child_pid),
        "queued GUI cleanup must not synchronously kill the owner child"
    );
    drain_queued_blocking_tasks(&tasks);
    assert!(
        !gwt::process::is_process_alive(child_pid),
        "owner cleanup ACK must follow exact child reap"
    );
    let session =
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("closed Session");
    assert!(
        !session.restore_window_on_startup,
        "stale completion must not restore a closed pane"
    );
    assert_eq!(session.status, gwt_agent::AgentStatus::Stopped);
    executor.block_on(owner.shutdown());
}

#[test]
fn launch_complete_workers_preserve_other_launches_when_finishing_out_of_order() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _process_home = ScopedEnvVar::set("HOME", temp.path());
    let _process_userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    init_repo_with_initial_commit(temp.path());
    gwt::agent_names::mutate_project_roster(
        temp.path(),
        &gwt_core::paths::gwt_sessions_dir(),
        gwt::agent_names::AgentRosterAction::Resize { size: 2 },
        None,
    )
    .expect("two simultaneous workers");
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::OwnerCommand;
        let owner = production_owner_fixture(temp.path()).await;
        let first_checkpoint = owner
            .pause_next_agent_launch()
            .await
            .expect("first worker checkpoint");
        let command = owner_fixture_agent_command(&owner, temp.path(), "first", "first-operation");
        let client = owner.client();
        let first = tokio::spawn(async move { client.command(command).await });
        let first_prepared = first_checkpoint.wait().await;
        let second_checkpoint = owner
            .pause_next_agent_launch()
            .await
            .expect("second worker checkpoint");
        let mut command =
            owner_fixture_agent_command(&owner, temp.path(), "second", "second-operation");
        if let OwnerCommand::LaunchAgent { config, .. } = &mut command {
            config.agent_id = gwt_agent::AgentId::Custom("fixture-second".into());
        }
        let client = owner.client();
        let mut second = tokio::spawn(async move { client.command(command).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the distinct real second worker checkpoint or an early canonical refusal
        let second_prepared = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::select! {
                captured = second_checkpoint.wait() => captured,
                result = &mut second => panic!("second distinct launch refused before preparation: {result:?}"),
            }
        }).await.unwrap();
        second_checkpoint.release();
        second
            .await
            .expect("second worker")
            .expect("second physical launch ACK");
        first_checkpoint.release();
        first
            .await
            .expect("first worker")
            .expect("first physical launch ACK");
        let projection = gwt_core::workspace_projection::load_workspace_projection(temp.path())
            .expect("load Work projection")
            .expect("Work projection exists");
        let sessions = projection
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect::<HashSet<_>>();
        assert!(sessions.contains(
            first_prepared
                .session_id
                .as_deref()
                .expect("first actual Session")
        ));
        assert!(
            sessions.contains(
                second_prepared
                    .session_id
                    .as_deref()
                    .expect("second actual Session")
            ),
            "worker A must preserve worker B's published agent"
        );
        for captured in [first_prepared, second_prepared] {
            owner
                .command(OwnerCommand::Close {
                    id: captured.id,
                    incarnation: captured.incarnation,
                })
                .await
                .expect("exact child cleanup");
        }
        owner.shutdown().await;
    });
}

#[test]
fn stale_prepared_cleanup_preserves_same_id_replacement_and_inflight_key() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let _process_home = ScopedEnvVar::set("HOME", temp.path());
    let _process_userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::OwnerCommand;
        let owner = production_owner_fixture(temp.path()).await;
        let checkpoint = owner
            .pause_next_agent_launch()
            .await
            .expect("prepared checkpoint");
        let command = owner_fixture_agent_command(&owner, temp.path(), "old-pane", "old-operation");
        let client = owner.client();
        let launch = tokio::spawn(async move { client.command(command).await });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner preparation checkpoint; the checkpoint establishes ordering
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .expect("actual preparation reaches the checkpoint");
        let original = captured.session.as_ref().expect("actual prepared Session");
        let pty = captured.pty.as_ref().expect("actual prepared PTY");
        let child_pid = pty.process_id().expect("actual child PID");
        assert!(gwt::process::is_process_alive(child_pid));
        let session_path =
            gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", original.id));
        let mut replacement = original.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("replacement".into());
        replacement
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("same-id replacement");
        let replacement_bytes = fs::read(&session_path).expect("replacement bytes");

        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "agent-1",
            temp.path().into(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(&temp.path().join(".gwt"), vec![tab], Some("tab-1"));
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        let window_id = combined_window_id("tab-1", "agent-1");
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("feature/test")
            .build();
        config.working_dir = Some(temp.path().into());
        let key = super::super::launch::inflight_launch_key("tab-1", &config).expect("dedup key");
        let replacement_launch = (window_id.clone(), Instant::now());
        runtime
            .inflight_launches
            .insert(key.clone(), replacement_launch.clone());
        let generation = runtime
            .window_lifecycle_generations
            .lock()
            .unwrap()
            .get(&window_id)
            .copied();
        runtime
            .queue_project_owner_launch(
                &runtime.test_context(),
                window_id.clone(),
                generation,
                Box::new(config),
                Box::default(),
            )
            .expect("next immutable GUI intent");
        let queued = tasks.lock().unwrap().len();

        let client = owner.client();
        let id = captured.id.clone();
        let operation_id = captured.operation_id.clone();
        let cancel = tokio::spawn(async move {
            client
                .command(OwnerCommand::PendingCancel { id, operation_id })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for observing the actual owner cancellation flag, not a timing assertion
        tokio::time::timeout(Duration::from_secs(30), async {
            while captured.current.load(std::sync::atomic::Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("exact cancellation admission");
        checkpoint.release();
        assert!(launch.await.expect("launch worker").is_err());
        assert!(
            cancel.await.expect("cancel worker").is_err(),
            "same-id replacement must refuse stale durable finalization"
        );
        assert!(
            !gwt::process::is_process_alive(child_pid),
            "stale handoff must reap its captured child"
        );
        assert!(runtime.pending_launch_completions.contains_key(&window_id));
        assert_eq!(
            runtime.inflight_launches.get(&key),
            Some(&replacement_launch)
        );
        assert_eq!(
            tasks.lock().unwrap().len(),
            queued,
            "old physical cleanup must not consume the next GUI launch"
        );
        assert_eq!(fs::read(&session_path).unwrap(), replacement_bytes);
        assert!(owner
            .command(OwnerCommand::PendingCancel {
                id: captured.id.clone(),
                operation_id: captured.operation_id.clone(),
            })
            .await
            .is_err());
        assert_eq!(
            fs::read(&session_path).unwrap(),
            replacement_bytes,
            "replayed cleanup must remain fenced"
        );

        // Restore only this fixture's captured identity to drain its retained exact
        // cleanup obligation after all replacement-preservation assertions.
        original
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("fixture cleanup identity");
        owner
            .command(OwnerCommand::Close {
                id: captured.id,
                incarnation: captured.incarnation,
            })
            .await
            .expect("retry the captured owner cleanup");
        owner.shutdown().await;
    });
}

#[test]
fn issue_5642_old_launch_completion_cannot_replace_another_issue_reusing_its_window_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let _process_home = ScopedEnvVar::set("HOME", temp.path());
    let _process_userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    let owner = executor.block_on(production_owner_fixture(temp.path()));
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().into(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded) =
        sample_runtime_with_events(&temp.path().join(".gwt"), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.tabs[0]
        .workspace
        .set_linked_issue_number("agent-1", Some(42));
    let gwt::project_runtime::events::OwnerCommand::LaunchAgent {
        config, metadata, ..
    } = owner_fixture_agent_command(&owner, temp.path(), "old-issue", "old-issue-launch")
    else {
        unreachable!("agent fixture command")
    };
    let generation = runtime
        .window_lifecycle_generations
        .lock()
        .unwrap()
        .get(&window_id)
        .copied();
    let context = runtime.test_context();
    runtime
        .queue_project_owner_launch(&context, window_id.clone(), generation, config, metadata)
        .expect("queue original owner launch");
    drain_queued_blocking_tasks(&tasks);
    let old_completion = take_remote_owner_launch_completion(&recorded);
    assert!(
        old_completion.result.is_ok(),
        "original owner accepted the physical launch"
    );

    runtime.close_window_after_issue_monitor_finalize_events(&window_id);
    drain_queued_blocking_tasks(&tasks);
    let new_window = runtime.tabs[0].workspace.add_window(WindowPreset::Agent);
    assert_eq!(combined_window_id("tab-1", &new_window.id), window_id);
    runtime.register_window("tab-1", &new_window.id);
    let mut new_session =
        gwt_agent::Session::new(temp.path(), "work/issue-43", gwt_agent::AgentId::Codex);
    new_session.linked_issue_number = Some(43);
    new_session.save(&runtime.sessions_dir).unwrap();
    runtime.tabs[0]
        .workspace
        .set_linked_issue_number(&new_window.id, Some(43));
    runtime.tabs[0]
        .workspace
        .set_session_id(&new_window.id, Some(new_session.id.clone()));
    let mut feedback = issue_monitor_feedback(43);
    feedback.issue_monitor_project_root = Some(temp.path().into());
    runtime
        .pending_launch_feedback_contexts
        .insert(window_id.clone(), feedback);

    assert!(runtime
        .handle_remote_owner_launch_complete(old_completion)
        .is_empty());
    drain_queued_blocking_tasks(&tasks);
    let current_session = runtime.tabs[0]
        .workspace
        .window(&new_window.id)
        .unwrap()
        .session_id
        .clone();
    let pending_issue = runtime
        .pending_launch_feedback_contexts
        .get(&window_id)
        .and_then(|feedback| feedback.issue_monitor_issue_number);
    assert_eq!(
        current_session,
        Some(new_session.id),
        "an old completion must not install its Session in another Issue's replacement pane"
    );
    assert_eq!(pending_issue, Some(43), "the new launch keeps its feedback");
    assert!(!runtime.runtimes.contains_key(&window_id));
    executor.block_on(owner.shutdown());
}

#[test]
fn launch_complete_defers_all_session_and_pty_work() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().into(),
        ProjectKind::Git,
        &[
            WindowPreset::Agent,
            WindowPreset::Agent,
            WindowPreset::Agent,
        ],
    );
    let window_ids: Vec<_> = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| combined_window_id("tab-1", &window.id))
        .collect();
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let context = runtime.test_context();
    for (index, window_id) in window_ids.into_iter().enumerate() {
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch(format!("work/test-{index}"))
            .working_dir(temp.path())
            .build();
        config.command = "nonexistent-launch-complete-regression".into();
        let metadata = gwt::project_runtime::launch_payload::OwnerLaunchMetadata {
            launch_feedback_context: (index == 1).then(|| LaunchFeedbackContext {
                client_id: "__issue_monitor__".into(),
                title: "Review dispatch".into(),
                issue_monitor_issue_number: Some(5187),
                issue_monitor_delivery_id: None,
                issue_monitor_project_root: Some(temp.path().into()),
                issue_monitor_session_mode: Some(gwt_agent::SessionMode::Normal),
                issue_monitor_autonomous_handoff: None,
                issue_monitor_autonomous_submit_started: false,
                issue_monitor_review_dispatch: true,
            }),
            ..Default::default()
        };
        let generation = runtime
            .window_lifecycle_generations
            .lock()
            .unwrap()
            .get(&window_id)
            .copied();
        let events = runtime
            .queue_project_owner_launch(
                &context,
                window_id,
                generation,
                Box::new(config),
                Box::new(metadata),
            )
            .expect("queue immutable owner launch");
        assert!(events.is_empty(), "dispatch must only enqueue preparation");
    }
    assert_eq!(tasks.lock().unwrap().len(), 3);
    assert_eq!(runtime.pending_launch_completions.len(), 3);
    assert!(runtime.active_agent_sessions.is_empty());
    assert!(runtime.runtimes.is_empty());
}

#[test]
fn app_runtime_start_work_launch_completion_registers_multiple_unassigned_agents() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let worktree_one = temp.path().join("repo-work-20260504-1234");
    let worktree_two = temp.path().join("repo-work-20260504-1235");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    run_git(&repo, &["branch", "-M", "main"]);
    run_git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    for (branch, worktree) in [
        ("work/20260504-1234", &worktree_one),
        ("work/20260504-1235", &worktree_two),
    ] {
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                worktree.to_str().unwrap(),
            ],
        );
    }
    let mut persisted = empty_workspace_state();
    for id in ["agent-1", "agent-2"] {
        persisted.windows.push(sample_window(
            id,
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        ));
    }
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-1".into(),
        title: "Repo".into(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowRegistry::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (mut runtime, recorded) =
        sample_runtime_with_events(&temp.path().join(".gwt"), vec![tab], Some("tab-1"));
    gwt::agent_names::mutate_project_roster(
        &repo,
        &runtime.sessions_dir,
        gwt::agent_names::AgentRosterAction::Resize { size: 2 },
        None,
    )
    .expect("two admitted physical agents");
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    let owner = executor.block_on(production_owner_fixture(&repo));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let mut actual_sessions = Vec::new();
    let mut physical = Vec::new();
    let mut second_events = Vec::new();
    for (raw, branch, worktree) in [
        ("agent-1", "work/20260504-1234", &worktree_one),
        ("agent-2", "work/20260504-1235", &worktree_two),
    ] {
        let gwt::project_runtime::events::OwnerCommand::LaunchAgent { mut config, .. } =
            owner_fixture_agent_command(&owner, &repo, raw, raw)
        else {
            unreachable!("agent fixture")
        };
        config.working_dir = Some(worktree.clone());
        config.branch = Some(branch.into());
        config.base_branch = Some("origin/main".into());
        let window_id = combined_window_id("tab-1", raw);
        let generation = runtime
            .window_lifecycle_generations
            .lock()
            .unwrap()
            .get(&window_id)
            .copied();
        runtime
            .queue_project_owner_launch(
                &runtime.test_context(),
                window_id.clone(),
                generation,
                config,
                Box::default(),
            )
            .expect("immutable unassigned Start Work launch");
        drain_queued_blocking_tasks(&tasks);
        let completion = take_remote_owner_launch_completion(&recorded);
        match &completion.result {
            Ok(super::super::RemoteOwnerLaunchOutcome::Runtime(snapshot, _)) => {
                actual_sessions.push(snapshot.session_id.clone().expect("actual Session"));
                physical.push((snapshot.id.clone(), snapshot.incarnation));
            }
            _ => panic!("actual owner must accept the physical launch"),
        }
        second_events = runtime.handle_remote_owner_launch_complete(completion);
    }
    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    let session_ids = projection
        .agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(projection.agents.len(), 2);
    assert!(session_ids.contains(actual_sessions[0].as_str()));
    assert!(session_ids.contains(actual_sessions[1].as_str()));
    assert!(projection
        .agents
        .iter()
        .all(gwt_core::workspace_projection::WorkspaceAgentSummary::is_unassigned));
    assert!(
        second_events
            .iter()
            .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "launch completion must not rebuild the projection inline"
    );
    let refreshed = drain_active_work_projection_refresh(&mut runtime, &repo);
    assert!(refreshed.iter().any(|event| matches!(event, OutboundEvent { target: DispatchTarget::Project(_),
        event: BackendEvent::ActiveWorkProjection { projection }, .. } if projection.active_agents == 2
            && projection.active_work_count == 2 && projection.agents.len() == 2 && projection.unassigned_agents.is_empty())));
    executor.block_on(async {
        for (id, incarnation) in physical {
            owner
                .command(gwt::project_runtime::events::OwnerCommand::Close { id, incarnation })
                .await
                .expect("exact cleanup");
        }
        owner.shutdown().await;
    });
}

#[test]
fn app_runtime_active_work_projection_groups_live_assigned_agents_by_work_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-a", Some("work-a"));
        agent.window_id = Some("tab-1::agent-a".to_string());
        agent.branch = Some("work/a".to_string());
        agent.title_summary = Some("Parser cleanup".to_string());
        agent
    });
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-b", Some("work-b"));
        agent.window_id = Some("tab-1::agent-b".to_string());
        agent.branch = Some("work/b".to_string());
        agent.title_summary = Some("UI polish".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (window_id, session_id, branch) in [
        ("tab-1::agent-a", "session-a", "work/a"),
        ("tab-1::agent-b", "session-b", "work/b"),
    ] {
        let mut session = sample_active_agent_session("tab-1", window_id);
        session.session_id = session_id.to_string();
        session.branch_name = branch.to_string();
        session.window_id = window_id.to_string();
        runtime
            .active_agent_sessions
            .insert(window_id.to_string(), session);
    }
    // SPEC-2359 Phase W-12 Slice 2 (FR-348): Work identity is
    // `agent_session_id`-derived, so each live session owns its own row.
    let expected_a = "work-session-session-a";
    let expected_b = "work-session-session-b";

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 2);
    assert_eq!(view.active_works.len(), 2);
    assert_eq!(view.active_works[0].agents.len(), 1);
    assert_eq!(view.active_works[1].agents.len(), 1);
    assert!(view.active_works.iter().any(|work| work.id == expected_a
        && work
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-a")));
    assert!(view.active_works.iter().any(|work| work.id == expected_b
        && work
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-b")));
}

#[test]
fn app_runtime_active_work_projection_includes_managed_hook_health() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let stable = temp
        .path()
        .join(format!("stable/gwtd{}", std::env::consts::EXE_SUFFIX));
    write_executable_test_file(&stable, "stable");
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &stable);
    gwt_skills::generate_codex_hooks(&repo).expect("generate codex hooks");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-a");
    session.session_id = "session-a".to_string();
    session.worktree_path = repo.clone();
    session.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-a".to_string(), session);
    let runtime_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, "session-a");
    gwt::cli::hook::runtime_state::write_for_event(&runtime_path, "PreToolUse")
        .expect("runtime state");

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    let health = view
        .managed_hook_health
        .as_ref()
        .expect("managed hook health");
    assert_eq!(health.status, "ready");
    assert_eq!(health.last_event.as_deref(), Some("PreToolUse"));
    assert!(health.issues.is_empty(), "{:?}", health.issues);
    let row_health = view.active_works[0]
        .managed_hook_health
        .as_ref()
        .expect("row managed hook health");
    assert_eq!(row_health.status, "ready");
    assert_eq!(row_health.last_event.as_deref(), Some("PreToolUse"));
}

#[test]
fn managed_hook_health_for_saved_row_ignores_ambient_session_runtime_state() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("saved-worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let stable = temp
        .path()
        .join(format!("stable/gwtd{}", std::env::consts::EXE_SUFFIX));
    write_executable_test_file(&stable, "stable");
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &stable);
    gwt_skills::generate_codex_hooks(&worktree).expect("generate hooks");
    let foreign_runtime_path = temp.path().join("foreign-runtime-state.json");
    gwt::cli::hook::runtime_state::write_for_event(&foreign_runtime_path, "PreToolUse")
        .expect("runtime state");
    let _runtime_path = ScopedEnvVar::set(
        gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
        &foreign_runtime_path,
    );

    let health = super::super::workspace_views::managed_hook_health_view_for_worktree(
        &worktree,
        temp.path(),
        &[],
        &gwt::cli::hook::health::ManagedHookFailureSnapshot::read(),
    );

    assert!(health.is_none(), "{health:?}");
}

#[test]
fn managed_hook_health_for_worktree_uses_the_latest_matching_session_state() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let stable = temp
        .path()
        .join(format!("stable/gwtd{}", std::env::consts::EXE_SUFFIX));
    write_executable_test_file(&stable, "stable");
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &stable);
    gwt_skills::generate_codex_hooks(&worktree).expect("generate hooks");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    for (session_id, updated_at, event) in [
        ("session-z", "2026-08-03T10:00:00Z", "PreToolUse"),
        ("session-a", "2026-08-03T11:00:00Z", "UserPromptSubmit"),
    ] {
        let path = gwt_agent::runtime_state_path(&sessions_dir, session_id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "status": "Running",
                "updated_at": updated_at,
                "last_activity_at": updated_at,
                "source_event": event,
                "pending_discussion": null
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let mut first = sample_active_agent_session("tab-1", "tab-1::agent-z");
    first.session_id = "session-z".to_string();
    first.worktree_path = worktree.clone();
    let mut second = sample_active_agent_session("tab-1", "tab-1::agent-a");
    second.session_id = "session-a".to_string();
    second.worktree_path = worktree.clone();

    let health = super::super::workspace_views::managed_hook_health_view_for_worktree(
        &worktree,
        &sessions_dir,
        &[&first, &second],
        &gwt::cli::hook::health::ManagedHookFailureSnapshot::read(),
    )
    .expect("managed hook health");

    assert_eq!(health.last_event.as_deref(), Some("UserPromptSubmit"));
    // The selection timestamp must also reuse the unchanged runtime snapshot,
    // even when another session has a valid but older timestamp.
    let latest = gwt_agent::runtime_state_path(&sessions_dir, &second.session_id);
    let metadata = fs::metadata(&latest).unwrap();
    fs::write(&latest, vec![b'x'; metadata.len() as usize]).unwrap();
    fs::File::options()
        .write(true)
        .open(&latest)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    let reused = super::super::workspace_views::managed_hook_health_view_for_worktree(
        &worktree,
        &sessions_dir,
        &[&first, &second],
        &gwt::cli::hook::health::ManagedHookFailureSnapshot::read(),
    )
    .expect("cached hook health");
    assert_eq!(reused.last_event.as_deref(), Some("UserPromptSubmit"));
}

#[test]
fn managed_hook_health_for_one_hundred_forty_rows_reuses_session_json_within_one_second() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let _hook_bin = ScopedEnvVar::unset("GWT_HOOK_BIN");
    let sessions_dir = temp.path().join("sessions");
    let sessions = (0..140)
        .map(|index| {
            let mut session =
                sample_active_agent_session("tab-1", &format!("tab-1::agent-{index}"));
            session.session_id = format!("session-{index}");
            session.worktree_path = temp.path().join(format!("work-{index}"));
            // Keep health visible without unrelated Git/config audits in fixture setup.
            gwt::cli::hook::health::record_managed_hook_self_healed(&session.worktree_path)
                .unwrap();
            let path = gwt_agent::runtime_state_path(&sessions_dir, &session.session_id);
            gwt::cli::hook::runtime_state::write_for_event(&path, "PreToolUse").unwrap();
            session
        })
        .collect::<Vec<_>>();
    let project = || {
        let snapshot = gwt::cli::hook::health::ManagedHookFailureSnapshot::default();
        for session in &sessions {
            let health = super::super::workspace_views::managed_hook_health_view_for_worktree(
                &session.worktree_path,
                &sessions_dir,
                &[session],
                &snapshot,
            )
            .expect("Work row hook health");
            assert_eq!(health.last_event.as_deref(), Some("PreToolUse"));
        }
    };
    project(); // Populate the existing surface cache outside the runtime budget.
               // Replacing bytes while retaining each stamp proves every runtime is reused.
    for session in &sessions {
        let path = gwt_agent::runtime_state_path(&sessions_dir, &session.session_id);
        let metadata = fs::metadata(&path).unwrap();
        fs::write(&path, vec![b'x'; metadata.len() as usize]).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
            .unwrap();
    }
    let started = std::time::Instant::now();
    project();
    let elapsed = started.elapsed();
    eprintln!(
        "Work hook health: 140 rows, unchanged runtime JSON, {}ms",
        elapsed.as_millis()
    );
    assert!(
        elapsed <= std::time::Duration::from_secs(1),
        "140 rows took {elapsed:?}"
    );
}

#[test]
fn startup_self_heals_managed_hooks_in_every_known_worktree() {
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().expect("root");
    let first = root.path().join("first");
    let second = root.path().join("second");
    for worktree in [&first, &second] {
        let local = worktree.join(format!("target/debug/gwtd{}", std::env::consts::EXE_SUFFIX));
        fs::create_dir_all(local.parent().unwrap()).unwrap();
        run_git(worktree, &["init", "-q"]);
        fs::write(&local, "local").unwrap();
        let _local_hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &local);
        gwt_skills::generate_settings_local(worktree).unwrap();
        gwt_skills::generate_codex_hooks(worktree).unwrap();
        gwt_skills::generate_opencode_hooks(worktree).unwrap();
        gwt_skills::generate_openclaw_hooks(worktree).unwrap();
        gwt_skills::generate_hermes_hooks(worktree).unwrap();
    }
    let stable = root
        .path()
        .join(format!("stable/gwtd{}", std::env::consts::EXE_SUFFIX));
    write_executable_test_file(&stable, "stable");
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &stable);

    super::super::startup::self_heal_managed_hooks_in_worktrees([
        first.as_path(),
        second.as_path(),
    ]);

    // Reproduce the second half of the historical failure: after startup
    // self-heal converges every provider, an old/debug GUI rematerializes the
    // same worktree while carrying its checkout runtime in GWT_BIN_PATH. The
    // stable generation fallback must remain unchanged.
    for worktree in [&first, &second] {
        let local = worktree.join("target/debug/gwtd");
        let _runtime_bin = ScopedEnvVar::set("GWT_BIN_PATH", &local);
        refresh_managed_gwt_assets_for_worktree(worktree).expect("old/debug GUI rematerialization");
    }

    for worktree in [&first, &second] {
        let local = worktree.join(format!("target/debug/gwtd{}", std::env::consts::EXE_SUFFIX));
        for relative in [
            ".claude/settings.local.json",
            ".codex/hooks.json",
            ".gwt/opencode/plugins/gwt-hooks.js",
            ".gwt/openclaw/plugins/gwt-hook-bridge/plugin.ts",
            ".gwt/hermes/agent-hooks/gwt-hook.sh",
        ] {
            let rendered =
                managed_hook_inspection_text(&fs::read_to_string(worktree.join(relative)).unwrap());
            assert!(rendered.contains("GWT_BIN_PATH"), "{relative}: {rendered}");
            assert!(
                !rendered.contains(&local.display().to_string()),
                "{relative}: {rendered}"
            );
        }
        let health = gwt::cli::hook::health::read_managed_hook_health(
            &gwt::cli::hook::health::ManagedHookHealthInput::new(worktree),
        );
        assert_eq!(
            health.status,
            gwt::cli::hook::health::ManagedHookHealthStatus::SelfHealed
        );
    }
}

/// Issue #4825 / SPEC-2359 FR-346 retirement: startup keeps saved agent
/// identity intact whether the obsolete reset marker is absent or stale.
#[test]
fn bootstrap_preserves_agent_identity_without_running_retired_reset() {
    use gwt_core::workspace_projection::{
        load_workspace_projection_from_path, save_workspace_projection_to_path, WorkspaceProjection,
    };

    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for marker in [None, Some(br#"{"version":0}"#.as_slice())] {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("repo dir");
        run_git(&repo, &["init", "-q"]);
        let current = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo);
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        let mut agent = workspace_agent_summary_for_test("saved-identity", None);
        agent.title_summary = Some("Saved purpose".to_string());
        agent.current_focus = Some("Saved progress".to_string());
        projection.agents.push(agent);
        save_workspace_projection_to_path(&current, &projection).expect("save identity");
        let marker_path = current.with_file_name("agent_identity.migration.json");
        if let Some(bytes) = marker {
            fs::write(&marker_path, bytes).expect("stale reset marker");
        }
        let tab = sample_project_tab("tab-repo", "Repo", repo, ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
        let (spawner, _tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;

        runtime.bootstrap();

        let loaded = load_workspace_projection_from_path(&current)
            .expect("load identity after startup")
            .expect("projection retained");
        assert_eq!(
            loaded.agents, projection.agents,
            "startup must retain identity"
        );
        match marker {
            Some(bytes) => assert_eq!(
                fs::read(&marker_path).expect("retired marker retained"),
                bytes,
                "startup must not rewrite a retired marker"
            ),
            None => assert!(
                !marker_path.exists(),
                "startup must not create a reset marker"
            ),
        }
    }
}

/// Issue #3808 AC-4: the worktree-wide managed hook self-heal audited 203
/// worktrees for 191 s on the startup path, ahead of the embedded server
/// bind. Launches refresh the managed assets of the worktree they start in,
/// so the sweep is a repair rather than a launch precondition: it runs on the
/// blocking worker and never delays the first frame.
#[test]
fn bootstrap_runs_managed_hook_self_heal_off_the_startup_path() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    run_git(&repo, &["init", "-q"]);
    let missing_pin = temp
        .path()
        .join(format!("missing/gwtd{}", std::env::consts::EXE_SUFFIX));
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &missing_pin);
    let config = repo.join(".codex/hooks.json");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let legacy = r#"{"hooks":{"SessionStart":[{"matcher":"*","hooks":[{"type":"command","command":"/repo/target/debug/gwtd hook event SessionStart"}]}]}}"#;
    fs::write(&config, legacy).unwrap();
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.bootstrap();

    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        legacy,
        "the self-heal sweep must not run synchronously inside bootstrap"
    );
    let queued = std::mem::take(
        &mut *tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    assert!(
        !queued.is_empty(),
        "bootstrap must schedule the sweep on the blocking worker"
    );
    for task in queued {
        task();
    }
    let healed = fs::read_to_string(&config).unwrap();
    assert!(
        managed_hook_inspection_text(&healed).contains("GWT_BIN_PATH"),
        "the deferred sweep must still heal the worktree: {healed}"
    );
    assert!(repo.join(".gwt/managed-hook-self-healed").exists());
}

#[test]
fn startup_self_heal_converges_legacy_config_to_explicit_hook_binary_pin() {
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    let missing_pin = root
        .path()
        .join(format!("missing/gwtd{}", std::env::consts::EXE_SUFFIX));
    let config = worktree.join(".codex/hooks.json");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        r#"{"hooks":{"SessionStart":[{"matcher":"*","hooks":[{"type":"command","command":"/repo/target/debug/gwtd hook event SessionStart"}]}]}}"#,
    )
    .unwrap();
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &missing_pin);

    super::super::startup::self_heal_managed_hooks_in_worktrees([worktree.as_path()]);

    let rendered = fs::read_to_string(&config).unwrap();
    assert!(
        managed_hook_inspection_text(&rendered).contains("GWT_BIN_PATH"),
        "{rendered}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&rendered).expect("parse healed hooks");
    let session_start_command = parsed
        .pointer("/hooks/SessionStart/0/hooks/0/command")
        .and_then(serde_json::Value::as_str)
        .expect("SessionStart command");
    let inspected_command = gwt_skills::decode_powershell_encoded_command(session_start_command)
        .unwrap_or_else(|| session_start_command.to_string());
    let normalized_command = inspected_command.replace('\\', "/");
    let normalized_pin = missing_pin.display().to_string().replace('\\', "/");
    assert!(normalized_command.contains(&normalized_pin), "{rendered}");
    assert!(
        !managed_hook_inspection_text(&rendered).contains("/repo/target/debug/gwtd"),
        "{rendered}"
    );
    assert!(worktree.join(".gwt/managed-hook-self-healed").exists());
}

#[test]
fn startup_self_heal_does_not_rewrite_canonical_config_for_missing_explicit_pin() {
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    let missing_pin = root
        .path()
        .join(format!("missing/gwtd{}", std::env::consts::EXE_SUFFIX));
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &missing_pin);
    gwt_skills::generate_codex_hooks(&worktree).expect("generate hooks");
    let config = worktree.join(".codex/hooks.json");
    let before = fs::read(&config).unwrap();

    super::super::startup::self_heal_managed_hooks_in_worktrees([worktree.as_path()]);

    assert_eq!(fs::read(&config).unwrap(), before);
    assert!(!worktree.join(".gwt/managed-hook-self-healed").exists());
}

#[test]
fn startup_self_heal_ignores_ambient_corrupt_runtime_state() {
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    let stable = root
        .path()
        .join(format!("stable/gwtd{}", std::env::consts::EXE_SUFFIX));
    write_executable_test_file(&stable, "stable");
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &stable);
    gwt_skills::generate_codex_hooks(&worktree).expect("generate hooks");
    let config = worktree.join(".codex/hooks.json");
    let before = fs::read(&config).unwrap();
    let runtime_path = root.path().join("runtime-state.json");
    fs::write(&runtime_path, "not-json").unwrap();
    let _runtime_path = ScopedEnvVar::set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);

    super::super::startup::self_heal_managed_hooks_in_worktrees([worktree.as_path()]);

    assert_eq!(fs::read(&config).unwrap(), before);
    assert!(!worktree.join(".gwt/managed-hook-self-healed").exists());
}

/// #3474: the `.codex/hooks.json` committed before the guarded template landed
/// reports ONLY `managed hook binary missing:` when its bare fallback cannot be
/// resolved, so the loop breaker below skipped it on every launch and the file
/// never converged. A missing host-native runtime guard is now its own issue
/// class, so a legacy config is repaired — and the repaired file no longer
/// raises it, so this still cannot loop.
#[test]
fn startup_self_heal_converges_legacy_config_without_a_runtime_guard() {
    // Issue #3609: the other 387 acquisitions in this binary recover from a
    // poisoned mutex. `expect` here turned any unrelated panic under the lock
    // into a second, misleading failure in the same run.
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    let fixture_id = root
        .path()
        .file_name()
        .expect("temporary root basename")
        .to_string_lossy();
    let missing_pin = PathBuf::from(format!(
        r"C:\Users\AKIOJI~1\AppData\Local\Temp\gwt self-heal {fixture_id}\missing\gwtd.exe"
    ));
    assert!(
        !missing_pin.exists(),
        "the synthetic Windows fixture must remain unresolved: {}",
        missing_pin.display()
    );
    let config = worktree.join(".codex/hooks.json");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    // The legacy template, pinned to a binary that no longer exists: the ONLY
    // issue it can raise through the binary audit is `managed hook binary
    // missing:`, which is exactly what the loop breaker skips.
    let legacy = serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "gwt_bin=\"${{GWT_BIN_PATH:-{}}}\"; \"$gwt_bin\" hook event SessionStart",
                        missing_pin.display()
                    )
                }]
            }]
        }
    });
    fs::write(&config, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
    serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&config).unwrap())
        .expect("legacy managed-hook fixture must be valid JSON");
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &missing_pin);
    let expected_hook_bin = missing_pin.display().to_string();
    let resolution_context = || {
        format!(
            "expected={expected_hook_bin:?}, selected={:?}, thread_override={:?}, \
             GWT_HOOK_BIN={:?}, GWT_BIN_PATH={:?}, PATH={:?}",
            gwt::managed_assets::managed_hook_bin(),
            gwt_skills::settings_local::hook_bin_override(),
            std::env::var_os("GWT_HOOK_BIN"),
            std::env::var_os("GWT_BIN_PATH"),
            std::env::var_os("PATH"),
        )
    };
    let before_heal = resolution_context();
    let mut health_input = gwt::cli::hook::health::ManagedHookHealthInput::new(&worktree);
    health_input.runtime_state_path = None;
    health_input.expected_hook_bin = Some(expected_hook_bin.clone());
    let legacy_health = gwt::cli::hook::health::read_managed_hook_health(&health_input);
    assert!(
        legacy_health
            .issues
            .iter()
            .any(|issue| issue.starts_with("managed hook runtime guard missing:")),
        "{:?}",
        legacy_health.issues
    );

    super::super::startup::self_heal_managed_hooks_in_worktrees_with_expected(
        [worktree.as_path()],
        Some(&expected_hook_bin),
    );

    let healed_health = gwt::cli::hook::health::read_managed_hook_health(&health_input);
    assert!(
        !healed_health
            .issues
            .iter()
            .any(|issue| issue.starts_with("managed hook runtime guard missing:")),
        "{:?}; before: {before_heal}; after: {}",
        healed_health.issues,
        resolution_context()
    );
    assert!(
        !healed_health.issues.is_empty()
            && healed_health
                .issues
                .iter()
                .all(|issue| issue.starts_with("managed hook binary missing:")),
        "{:?}; before: {before_heal}; after: {}",
        healed_health.issues,
        resolution_context()
    );

    // A second pass over the converged file must be a no-op: the guard issue is
    // gone and only the unresolvable pin remains, which the loop breaker skips.
    let converged = fs::read(&config).unwrap();
    super::super::startup::self_heal_managed_hooks_in_worktrees_with_expected(
        [worktree.as_path()],
        Some(&expected_hook_bin),
    );
    assert_eq!(fs::read(&config).unwrap(), converged);
}

#[test]
fn startup_self_heal_does_not_loop_on_missing_current_literal_fallback() {
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    {
        let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", "gwtd");
        gwt_skills::generate_codex_hooks(&worktree).expect("generate hooks");
    }
    let config = worktree.join(".codex/hooks.json");
    let before = fs::read(&config).unwrap();
    let _path = ScopedEnvVar::set("PATH", "");

    super::super::startup::self_heal_managed_hooks_in_worktrees_with_expected(
        [worktree.as_path()],
        Some("gwtd"),
    );

    assert_eq!(fs::read(&config).unwrap(), before);
    assert!(!worktree.join(".gwt/managed-hook-self-healed").exists());
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): "1 agent session : 1 Work". When
/// the *same* `session_id` surfaces under multiple windows, the agents
/// collapse into a single Work row keyed by that session. The Workspace detail
/// is then normalized to the latest visible entry per agent identity.
#[test]
fn app_runtime_active_work_projection_groups_same_session_windows_in_one_work_row() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    for window_id in ["tab-1::agent-a", "tab-1::agent-b"] {
        let mut agent = workspace_agent_summary_for_test("session-shared", Some("work-shared"));
        agent.window_id = Some(window_id.to_string());
        agent.branch = Some("work/shared".to_string());
        projection.agents.push(agent);
    }
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for window_id in ["tab-1::agent-a", "tab-1::agent-b"] {
        let mut session = sample_active_agent_session("tab-1", window_id);
        session.session_id = "session-shared".to_string();
        session.branch_name = "work/shared".to_string();
        session.window_id = window_id.to_string();
        runtime
            .active_agent_sessions
            .insert(window_id.to_string(), session);
    }

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 1);
    assert_eq!(view.active_works.len(), 1);
    assert_eq!(view.active_works[0].id, "work-session-session-shared");
    assert_eq!(view.active_works[0].agents.len(), 1);
    assert!(view.active_works[0]
        .agents
        .iter()
        .all(|agent| agent.session_id == "session-shared"));
    assert_eq!(
        view.active_works[0].session_agent_total, 2,
        "hidden same-agent candidates stay counted for the session summary"
    );
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): `agent_session_id` is the storage
/// identity, but the Workspace detail shows only the latest visible entry per
/// agent identity after same-branch Works are grouped into one row.
#[test]
fn app_runtime_active_work_projection_separates_sessions_on_same_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    for (session_id, window_id) in [
        ("session-a", "tab-1::agent-a"),
        ("session-b", "tab-1::agent-b"),
    ] {
        let mut agent = workspace_agent_summary_for_test(session_id, Some("work-shared"));
        agent.window_id = Some(window_id.to_string());
        agent.branch = Some("work/shared".to_string());
        agent.updated_at = if session_id == "session-a" {
            Utc.with_ymd_and_hms(2026, 6, 17, 9, 0, 0).unwrap()
        } else {
            Utc.with_ymd_and_hms(2026, 6, 17, 10, 0, 0).unwrap()
        };
        projection.agents.push(agent);
    }
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (window_id, session_id) in [
        ("tab-1::agent-a", "session-a"),
        ("tab-1::agent-b", "session-b"),
    ] {
        let mut session = sample_active_agent_session("tab-1", window_id);
        session.session_id = session_id.to_string();
        session.branch_name = "work/shared".to_string();
        session.window_id = window_id.to_string();
        runtime
            .active_agent_sessions
            .insert(window_id.to_string(), session);
    }

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    // SPEC-2359 W16-2 (FR-389 / SC-259) supersedes the original two-row
    // contract here: storage still keys one Work per agent session (FR-348),
    // but the VIEW groups same-branch Works into one Workspace row carrying
    // both live agents.
    assert_eq!(
        view.active_works.len(),
        1,
        "same branch groups into one row"
    );
    let row = &view.active_works[0];
    assert_eq!(
        row.agents.len(),
        1,
        "same agent identity collapses to the newest visible session"
    );
    assert!(row
        .agents
        .iter()
        .any(|agent| agent.session_id == "session-b"));
    assert!(!row
        .agents
        .iter()
        .any(|agent| agent.session_id == "session-a"));
    assert_eq!(
        row.session_agent_total, 2,
        "hidden same-agent candidates stay counted for the session summary"
    );
    assert_eq!(
        row.works.len(),
        2,
        "both launch-scoped Works remain addressable"
    );
    assert!(row
        .works
        .iter()
        .any(|work| work.id == "work-session-session-a"));
    assert!(row
        .works
        .iter()
        .any(|work| work.id == "work-session-session-b"));
    assert!(row.workspace_key.is_some());
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-349): each `active_works` item carries a
/// `lifecycle_state` derived from the agent-session Work lifecycle. Active
/// Work rows group a live, assigned, running agent session, so the wire
/// state is `"active"` and `closed_at` is None (agent stop alone never
/// closes a Work — FR-350).
#[test]
fn app_runtime_active_work_projection_sets_lifecycle_state_active() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-a", Some("work-a"));
        agent.window_id = Some("tab-1::agent-a".to_string());
        agent.branch = Some("work/a".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-a");
    session.session_id = "session-a".to_string();
    session.branch_name = "work/a".to_string();
    session.window_id = "tab-1::agent-a".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-a".to_string(), session);

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_works.len(), 1);
    assert_eq!(view.active_works[0].lifecycle_state, "active");
    assert_eq!(view.active_works[0].closed_at, None);
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-349): the `lifecycle_state` field is
/// back-compat — a serialized `ActiveWorkItemView` payload that predates the
/// field deserializes with `lifecycle_state = "active"` and `closed_at =
/// None` via the serde defaults.
#[test]
fn active_work_item_view_lifecycle_state_back_compat_default() {
    let legacy = serde_json::json!({
        "id": "work-1",
        "title": "Legacy Work",
        "status_category": "active",
        "status_text": "1 active agent",
        "summary": null,
        "owner": null,
        "next_action": null,
        "active_agents": 1,
        "blocked_agents": 0,
        "branch": "work/legacy",
        "worktree_path": null,
        "pr_number": null,
        "pr_url": null,
        "pr_state": null,
        "board_refs": [],
        "agents": []
    });
    let view: gwt::ActiveWorkItemView =
        serde_json::from_value(legacy).expect("deserialize legacy active work item");
    assert_eq!(
        serde_json::to_value(&view).unwrap()["linked_issue_numbers"],
        serde_json::json!([])
    );
    assert_eq!(view.lifecycle_state, "active");
    assert_eq!(view.closed_at, None);
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): `agent_session_id` is the primary
/// Work identity, taking priority over both the branch-derived
/// `canonical_work_id` and the raw `workspace_id`. The resulting Work id is
/// `work-session-<session_id>`, and the branch-derived id is *not* used when
/// a session is present.
#[test]
fn app_runtime_active_work_projection_uses_agent_session_id_over_branch_and_workspace_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let branch_derived_work_id =
        gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/test"), None)
            .expect("canonical work id");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-a", Some("legacy-work-id"));
        agent.window_id = Some("tab-1::agent-a".to_string());
        agent.branch = Some("work/test".to_string());
        agent.title_summary = Some("Parser cleanup".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-a");
    session.session_id = "session-a".to_string();
    session.branch_name = "work/test".to_string();
    session.window_id = "tab-1::agent-a".to_string();
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session);

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 1);
    assert_eq!(view.active_works[0].id, "work-session-session-a");
    assert_ne!(view.active_works[0].id, branch_derived_work_id);
    assert_ne!(view.active_works[0].id, "legacy-work-id");
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): when the owning agent session
/// stops, the Work must not vanish from the Work surface. It is retained as
/// a `paused` `active_works` row (keyed by the session-derived Work id) until
/// the user explicitly closes it. Agent stop alone never closes a Work.
#[test]
fn app_runtime_active_work_projection_retains_stopped_agent_work_as_paused() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-paused", Some("work-paused"));
        agent.window_id = Some("tab-1::agent-paused".to_string());
        agent.branch = Some("work/paused".to_string());
        agent.title_summary = Some("Paused persistence".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-paused");
    session.session_id = "session-paused".to_string();
    session.branch_name = "work/paused".to_string();
    session.worktree_path = repo.join("work/paused");
    session.window_id = "tab-1::agent-paused".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-paused".to_string(), session);

    // While the agent is live the Work is Active.
    let live_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("live projection view");
    assert_eq!(live_view.active_works.len(), 1);
    assert_eq!(live_view.active_works[0].lifecycle_state, "active");

    // Agent stops: session leaves active_agent_sessions and a paused marker
    // is persisted to the work history.
    let stopped = runtime
        .active_agent_sessions
        .remove("tab-1::agent-paused")
        .expect("captured stopped projection source");
    gwt::project_runtime::workspace::paused_work_record_task(&repo, &stopped, chrono::Utc::now())
        .expect("canonical production Work pause task")();
    assert!(!runtime
        .active_agent_sessions
        .contains_key("tab-1::agent-paused"));

    let paused_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("paused projection view");
    assert_eq!(
        paused_view.active_works.len(),
        1,
        "stopped agent Work must remain in active_works as paused"
    );
    let paused = &paused_view.active_works[0];
    assert_eq!(paused.id, "work-session-session-paused");
    assert_eq!(paused.lifecycle_state, "paused");
    assert_eq!(paused.closed_at, None);
    assert_eq!(paused.active_agents, 0);
    assert_eq!(paused.branch.as_deref(), Some("work/paused"));
}

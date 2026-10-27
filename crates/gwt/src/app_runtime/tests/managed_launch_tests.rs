use super::*;

pub(super) fn managed_owner_fixture(
    temp: &Path,
    with_monitor: bool,
) -> (
    PathBuf,
    Arc<gwt::project_runtime::test_support::ProjectOwnerHarness>,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
) {
    use super::pane_control_tests::{control_launch_config, control_owner_launch};
    let repo = temp.join("project");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "-c",
            "user.name=Work fixture",
            "-c",
            "user.email=work@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "Work fixture",
        ],
    );
    seed_owner_fixture_roster(&repo, 8);
    let owner = Arc::new(
        owner_fixture_block_on(async {
            if with_monitor {
                gwt::project_runtime::test_support::ProjectOwnerHarness::start_with_monitor(&repo)
                    .await
            } else {
                gwt::project_runtime::test_support::ProjectOwnerHarness::start(&repo).await
            }
        })
        .unwrap(),
    );
    let pm = control_owner_launch(&owner, "pm", control_launch_config(&repo, 900));
    let session = pm.session.as_ref().unwrap();
    gwt::pm_registry::try_register_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(&repo),
        gwt::pm_registry::PmRegistration {
            session_id: session.id.clone(),
            agent_id: session.agent_id.command().into(),
            worktree_path: session.worktree_path.display().to_string(),
            created_at: Some(session.created_at.to_rfc3339()),
            consecutive_crashes: 0,
            next_not_before: None,
        },
        |_| false,
    )
    .unwrap();
    let config = control_launch_config(&repo, 901);
    let ordinary = control_owner_launch(&owner, "ordinary", config);
    (repo, owner, pm, ordinary)
}

fn managed_close_work(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    caller: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
    work_id: &str,
    close_kind: &str,
) -> serde_json::Value {
    let client = owner_fixture_block_on(
        owner.agent_client(&format!("work-client-{}", uuid::Uuid::new_v4())),
    )
    .unwrap();
    let reply = owner_fixture_block_on(client.request(
        caller.grant.as_ref().unwrap().clone(),
        AgentFrontendRequest::CloseWork {
            work_id: work_id.into(),
            close_kind: close_kind.into(),
        },
    ))
    .unwrap();
    reply
        .events
        .into_iter()
        .find(|event| event["kind"] == "work_close_result")
        .expect("actual owner WorkCloseResult")
}

fn managed_close_member(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    member: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
) {
    let pid = member.pty.process_id().unwrap();
    owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::Close {
            id: member.id.clone(),
            incarnation: member.incarnation,
        }),
    )
    .unwrap();
    assert!(
        !gwt::process::is_process_alive(pid),
        "exact Work member child must be reaped before recovery"
    );
}

#[test]
fn registered_pm_can_audit_supersede_an_exact_dead_invalid_successor() {
    assert_registered_pm_successor_recovery("Issue #42", true);
}

#[test]
fn registered_pm_can_recover_a_foreign_owner_invalid_successor() {
    for terminal in [false, true] {
        assert_registered_pm_successor_recovery("Issue #43", terminal);
    }
}

fn assert_registered_pm_successor_recovery(occupied_owner: &str, terminal: bool) {
    use gwt_core::workspace_projection as work;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, owner, pm, ordinary) = managed_owner_fixture(temp.path(), false);
    run_git(&repo, &["checkout", "-b", "work/issue-42"]);
    let now = Utc::now();
    let predecessor_id = work::canonical_work_id(&repo, Some("work/issue-42"), None).unwrap();
    let occupied_id = work::successor_work_id(&predecessor_id);
    let mut start = work::WorkEvent::new(work::WorkEventKind::Start, &predecessor_id, now);
    start.owner = Some("Issue #42".to_string());
    start.execution_container = Some(work::WorkspaceExecutionContainerRef {
        branch: Some("work/issue-42".to_string()),
        worktree_path: Some(repo.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    work::record_workspace_work_event(&repo, start.clone()).unwrap();
    start.id = "invalid-parentless-start".to_string();
    start.work_item_id = occupied_id.clone();
    start.owner = Some(occupied_owner.to_string());
    let mut member_config = super::pane_control_tests::control_launch_config(&repo, 902);
    // This is the invalid successor's member, with no separate producing
    // Work. The actual owner still creates its Session, PTY and close proof.
    member_config.linked_issue_number = None;
    member_config.suppress_execution_control = true;
    member_config.base_branch = None;
    let member =
        super::pane_control_tests::control_owner_launch(&owner, "invalid-member", member_config);
    let member_session_id = member.session.as_ref().unwrap().id.clone();
    start.agent_session_id = Some(member_session_id.clone());
    work::record_workspace_work_event(&repo, start).unwrap();
    work::emit_workspace_done_event_if_absent(&repo, &predecessor_id, now).unwrap();
    if terminal {
        work::emit_workspace_done_event_if_absent(&repo, &occupied_id, now).unwrap();
    }
    assert_eq!(
        managed_close_work(&owner, &ordinary, &occupied_id, "superseded")["ok"],
        false
    );
    assert_eq!(
        managed_close_work(&owner, &pm, &occupied_id, "superseded")["ok"],
        false,
        "a real live member must refuse exact successor recovery"
    );
    managed_close_member(&owner, &member);
    let member_path = gwt_core::paths::gwt_sessions_dir().join(format!("{member_session_id}.toml"));
    let stopped_member = fs::read(&member_path).unwrap();
    fs::write(&member_path, b"malformed session").unwrap();
    assert_eq!(
        managed_close_work(&owner, &pm, &occupied_id, "superseded")["ok"],
        false
    );
    fs::write(member_path, stopped_member).unwrap();
    let recovered = managed_close_work(&owner, &pm, &occupied_id, "superseded");
    assert_eq!(
        recovered["ok"], true,
        "PM has a real exact-target recovery even for terminal invalid occupancy: {recovered}"
    );
    assert_eq!(recovered["changed"], true);
    let replay = managed_close_work(&owner, &pm, &occupied_id, "superseded");
    assert_eq!(replay["ok"], true);
    assert_eq!(replay["changed"], false);
    let items = work::load_workspace_work_items(&repo).unwrap().unwrap();
    let occupied = items
        .work_items
        .iter()
        .find(|item| item.id == occupied_id)
        .unwrap();
    assert_eq!(occupied.owner.as_deref(), Some(occupied_owner));
    assert_eq!(occupied.completed_at, terminal.then_some(now));
    assert!(occupied.is_terminal());
    assert_eq!(
        occupied
            .events
            .iter()
            .filter(|event| event.is_successor_supersession())
            .count(),
        1
    );
    assert!(occupied
        .events
        .iter()
        .filter(|event| event.is_successor_supersession())
        .all(|event| event.agent_session_id.is_none()));

    let session = ActiveAgentSession {
        session_id: "fresh-owner-session".to_string(),
        branch_name: "work/issue-42".to_string(),
        worktree_path: repo.clone(),
        ..sample_active_agent_session("tab-1", "tab-1::fresh")
    };
    let mut projection = work::WorkspaceProjection::default_for_project(&repo);
    let events = super::super::workspace::apply_workspace_launch_for_current_work(
        &repo,
        &mut projection,
        &items,
        &session,
        super::super::workspace::WorkspaceLaunchTransition {
            work_id: None,
            base_branch: None,
            linked_issue_number: Some(42),
            canonical_owner: Some(gwt::cli::execution_state::ExecutionOwnerKey {
                kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 42,
            }),
            resume_context: None,
            kind: WorkspaceLaunchProjectionKind::StartWork,
            live_session_ids: &HashSet::new(),
            now: Utc::now(),
        },
        true,
    )
    .expect("fresh activation follows the PM audit into a new owner lifetime");
    let expected_id = work::successor_work_id(&occupied_id);
    assert_eq!(projection.id, expected_id);
    let mut updated = items.clone();
    for event in events {
        updated.apply_event(event);
    }
    assert_eq!(
        work::current_work_id(&updated, &repo, Some("work/issue-42"), None),
        Some(expected_id)
    );
    assert_eq!(
        updated
            .work_items
            .iter()
            .find(|item| item.id == occupied_id),
        Some(occupied),
        "fresh activation preserves the retired foreign history"
    );
}

#[test]
fn registered_pm_workspace_close_uses_canonical_eligibility_and_current_membership() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (repo, owner, pm, ordinary) = managed_owner_fixture(temp.path(), false);
    let work_id = "work-session-stopped";
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        work_id,
        Some("Stopped work"),
        None,
        None,
        &[],
        Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/stopped".into()),
                worktree_path: Some(repo.join("stopped")),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        ),
        Some("stopped"),
        chrono::Utc::now(),
    )
    .unwrap();

    // The existing display-cache classifier keeps its unavailable/stale
    // contract. The owner command below recomputes canonical close facts.
    let unavailable =
        gwt::project_runtime::agent_dispatch::validate_work_close_target(false, false, None)
            .unwrap_err();
    assert!(unavailable.1);
    let loading =
        gwt::project_runtime::agent_dispatch::validate_work_close_target(false, true, None)
            .unwrap_err();
    assert!(loading.1);
    let view: gwt::ActiveWorkspaceWorkView = serde_json::from_value(serde_json::json!({
        "id":work_id,"title":"Stopped work","status_category":"paused","status_text":"Paused",
        "lifecycle_state":"paused","manual_close_allowed":true,"updated_at":"2026-10-01T00:00:00Z",
    }))
    .unwrap();
    assert!(
        gwt::project_runtime::agent_dispatch::validate_work_close_target(
            true,
            true,
            Some(view.clone())
        )
        .is_ok()
    );
    let mut remote_view = view;
    remote_view.manual_close_allowed = false;
    remote_view.close_blocked_reason = Some("remote_unknown".into());
    assert!(
        gwt::project_runtime::agent_dispatch::validate_work_close_target(
            true,
            true,
            Some(remote_view)
        )
        .is_err()
    );

    let denied = managed_close_work(&owner, &ordinary, work_id, "done");
    assert_eq!(denied["ok"], false);
    assert_eq!(denied["retryable"], false);
    let unknown = managed_close_work(&owner, &pm, "unknown-or-foreign-work", "done");
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["retryable"], false);

    // Missing local path and an unobserved branch are the canonical remote
    // uncertainty that the previous GUI fixture wrote into a cached row.
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        "remote-work",
        Some("Remote work"),
        None,
        None,
        &[],
        Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("remote/unobserved".into()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        ),
        None,
        chrono::Utc::now(),
    )
    .unwrap();
    let remote = managed_close_work(&owner, &pm, "remote-work", "done");
    assert_eq!(remote["ok"], false);

    let mut config = super::pane_control_tests::control_launch_config(&repo, 902);
    config.linked_issue_number = None;
    config.suppress_execution_control = true;
    config.base_branch = None;
    let member = super::pane_control_tests::control_owner_launch(&owner, "resumed-member", config);
    let member_session_id = member.session.as_ref().unwrap().id.clone();
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        work_id,
        None,
        None,
        None,
        &[],
        None,
        Some(&member_session_id),
        chrono::Utc::now(),
    )
    .unwrap();
    let live = managed_close_work(&owner, &pm, work_id, "done");
    assert_eq!(live["ok"], false);
    assert!(live["error"]
        .as_str()
        .unwrap()
        .contains("current member agent"));
    managed_close_member(&owner, &member);

    let session_path =
        gwt_core::paths::gwt_sessions_dir().join(format!("{member_session_id}.toml"));
    let exact_stopped = fs::read(&session_path).unwrap();
    fs::write(&session_path, b"malformed member Session").unwrap();
    let stale = managed_close_work(&owner, &pm, work_id, "done");
    assert_eq!(stale["ok"], false);
    assert_eq!(stale["retryable"], true);
    fs::write(&session_path, exact_stopped).unwrap();

    let closed = managed_close_work(&owner, &pm, work_id, "done");
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(closed["changed"], true);
    let items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .unwrap()
        .unwrap();
    let target = items
        .work_items
        .iter()
        .find(|work| work.id == work_id)
        .unwrap();
    assert!(target.is_terminal());
    assert!(!target.discarded);
    assert!(items
        .work_items
        .iter()
        .filter(|work| work.id != work_id)
        .all(|work| !work.is_terminal()));
}

fn managed_owner_config_at(repo: &Path, worktree: &Path, branch: &str) -> gwt_agent::LaunchConfig {
    let mut config = super::pane_control_tests::control_launch_config(repo, 990);
    config.working_dir = Some(worktree.to_path_buf());
    config.branch = Some(branch.into());
    config.base_branch = None;
    config.linked_issue_number = None;
    config.suppress_execution_control = true;
    config
}

fn managed_owner_session_at(
    repo: &Path,
    worktree: &Path,
    branch: &str,
) -> (
    gwt::project_runtime::test_support::ProjectOwnerHarness,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
) {
    seed_owner_fixture_roster(repo, 8);
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(repo),
    )
    .unwrap();
    let pane = super::pane_control_tests::control_owner_launch(
        &owner,
        "stopped-session",
        managed_owner_config_at(repo, worktree, branch),
    );
    (owner, pane)
}

fn managed_project_owner_projection(
    temp_root: &Path,
    repo: &Path,
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    active: &[&gwt::project_runtime::test_support::OwnerRuntimeProbe],
) -> AppRuntime {
    let mut tab = sample_project_tab("tab-1", "Repo", repo.to_path_buf(), ProjectKind::Git, &[]);
    tab.workspace = WindowRegistry::from_persisted(
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(repo))
            .expect("actual owner persisted canvas"),
    );
    let mut runtime = sample_runtime(temp_root, vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let context = runtime.test_context();
    for pane in active {
        let raw = pane.id.split_once("::").unwrap().1;
        let alias = combined_window_id("tab-1", raw);
        let snapshot = owner_fixture_block_on(owner.command(
            gwt::project_runtime::events::OwnerCommand::Snapshot {
                id: pane.id.clone(),
                incarnation: pane.incarnation,
            },
        ))
        .expect("actual owner snapshot for Work projection");
        runtime.install_remote_owner_projection(
            &context,
            &alias,
            owner.client(),
            serde_json::from_value(snapshot).unwrap(),
            pane.session.clone(),
        );
    }
    runtime
}

fn managed_exit_fixture(
    temp: &Path,
    exit_code: u8,
    output: &str,
) -> (
    gwt::project_runtime::test_support::ProjectOwnerHarness,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
    AppRuntime,
    tokio::sync::broadcast::Receiver<gwt::project_runtime::events::OwnerEvent>,
) {
    let repo = temp.join("exit-project");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    seed_owner_fixture_roster(&repo, 4);
    let owner = owner_fixture_block_on(production_owner_fixture(&repo));
    let events = owner.subscribe();
    let mut config = managed_owner_config_at(&repo, &repo, "main");
    if cfg!(windows) {
        config.command = "powershell".into();
        config.args = vec!["-NoProfile".into(), "-Command".into(), format!(
            "$null = [Console]::ReadLine(); [Console]::WriteLine($env:GWT_EXIT_FIXTURE_OUTPUT); exit {exit_code}"
        )];
    } else {
        config.command = "/bin/sh".into();
        config.args = vec!["-c".into(), format!(
            "stty -echo; IFS= read -r line; printf '%s\\n' \"$GWT_EXIT_FIXTURE_OUTPUT\"; exit {exit_code}"
        )];
    }
    config
        .env_vars
        .insert("GWT_EXIT_FIXTURE_OUTPUT".into(), output.into());
    let probe = super::pane_control_tests::control_owner_launch(&owner, "exit-agent", config);
    let mut runtime = managed_project_owner_projection(temp, &repo, &owner, &[&probe]);
    let context = runtime.test_context();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = owner.client().identity().cloned();
    (owner, probe, runtime, events)
}

fn finish_managed_exit(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    probe: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
    runtime: &mut AppRuntime,
    events: &mut tokio::sync::broadcast::Receiver<gwt::project_runtime::events::OwnerEvent>,
) -> Vec<OutboundEvent> {
    use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
    let context = runtime.test_context();
    let identity = owner.client().identity().unwrap().clone();
    owner_fixture_block_on(async {
        owner
            .command(OwnerCommand::Input {
                id: probe.id.clone(),
                incarnation: probe.incarnation,
                data: b"exit\n".to_vec(),
            })
            .await
            .expect("release the actual provider exit gate");
        let mut projected = Vec::new();
        loop {
            // test-hygiene: allow-wall-clock-deadline Bounds delivery of actual owner exit/reap acknowledgement; no scheduler ordering is asserted.
            let event = tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, events.recv())
                .await
                .expect("actual owner terminal event")
                .expect("owner event stream");
            let closed = matches!(&event, OwnerEvent::Closed { id, incarnation, error: None }
                if id == &probe.id && *incarnation == probe.incarnation);
            if let OwnerEvent::Closed {
                error: Some(error), ..
            } = &event
            {
                panic!("actual owner exit cleanup failed: {error}");
            }
            let terminal = matches!(&event, OwnerEvent::Status { id, incarnation, exit_confirmed: true, .. }
                if id == &probe.id && *incarnation == probe.incarnation);
            projected.extend(runtime.handle_project_owner_event(&context, 1, &identity, event));
            if terminal {
                // A nonzero provider exit may retain an actual diagnostic
                // runtime. End that fixture through the real close path.
                let inventory = owner.command(OwnerCommand::ListPanes).await.unwrap();
                if inventory["panes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|pane| pane["id"] == probe.id && pane["incarnation"] == probe.incarnation)
                {
                    owner
                        .command(OwnerCommand::Close {
                            id: probe.id.clone(),
                            incarnation: probe.incarnation,
                        })
                        .await
                        .expect("drain the actual retained diagnostic runtime");
                }
            }
            if closed {
                break;
            }
        }
        assert!(
            probe.pty.tree_exit_confirmed(),
            "the actual provider tree was reaped"
        );
        projected
    })
}

#[test]
fn ephemeral_intake_session_stop_removes_clean_worktree_and_emits_no_paused_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    let intake = temp.path().join(".intake-clean");
    gwt_git::WorktreeManager::new(&repo)
        .create_detached("HEAD", &intake)
        .expect("intake worktree");
    assert!(intake.exists());
    let (codex_config_path, codex_project_key) =
        seed_codex_project_trust_for_cleanup(&intake, temp.path());

    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (owner, pane) = managed_owner_session_at(&repo, &intake, "");
    super::pane_control_tests::control_close(&owner, &pane);
    assert!(
        pane.pty.try_wait().unwrap().is_some(),
        "cleanup follows the actual child reap"
    );
    let saved = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir()
            .join(format!("{}.toml", pane.session.as_ref().unwrap().id)),
    )
    .unwrap();
    assert_eq!(saved.status, gwt_agent::AgentStatus::Stopped);
    assert!(
        !intake.exists(),
        "clean intake worktree is removed when the session ends"
    );
    assert_eq!(
        codex_project_trust_level(&codex_config_path, &codex_project_key),
        None,
        "removing the managed worktree must revoke its Codex project trust"
    );
    let recorded = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .ok()
        .flatten();
    assert!(
        recorded.is_none_or(|projection| projection.work_items.is_empty()),
        "no Work event is recorded for an ephemeral intake session"
    );
}

#[test]
fn ephemeral_intake_session_stop_keeps_dirty_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    let intake = temp.path().join(".intake-dirty");
    gwt_git::WorktreeManager::new(&repo)
        .create_detached("HEAD", &intake)
        .expect("intake worktree");
    // Uncommitted work must not be destroyed.
    fs::write(intake.join("wip.txt"), "unsaved intake work").expect("write wip");
    let (codex_config_path, codex_project_key) =
        seed_codex_project_trust_for_cleanup(&intake, temp.path());

    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (owner, pane) = managed_owner_session_at(&repo, &intake, "");
    super::pane_control_tests::control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());

    assert!(
        intake.exists() && intake.join("wip.txt").exists(),
        "a dirty intake worktree is kept so uncommitted work is never lost"
    );
    assert_eq!(
        codex_project_trust_level(&codex_config_path, &codex_project_key).as_deref(),
        Some("trusted"),
        "retaining the worktree must retain its Codex project trust"
    );
}

#[test]
fn ephemeral_intake_cleanup_keeps_worktree_when_codex_trust_cannot_be_revoked() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);

    let intake = temp.path().join(".intake-invalid-codex-config");
    gwt_git::WorktreeManager::new(&repo)
        .create_detached("HEAD", &intake)
        .expect("intake worktree");
    let config_path = temp.path().join(".codex/config.toml");
    fs::create_dir_all(config_path.parent().expect("Codex config parent"))
        .expect("create Codex config parent");
    fs::write(&config_path, "projects = [\n").expect("write malformed Codex config");

    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (owner, pane) = managed_owner_session_at(&repo, &intake, "");
    super::pane_control_tests::control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());

    assert!(
        intake.exists(),
        "Codex config parse failure must prevent filesystem deletion"
    );
    assert_eq!(
        fs::read_to_string(config_path).expect("malformed config remains"),
        "projects = [\n"
    );
}

#[test]
fn docker_ephemeral_intake_cleanup_removes_exact_host_trust_and_preserves_unrelated_state() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);

    let intake = temp.path().join(".intake-docker");
    gwt_git::WorktreeManager::new(&repo)
        .create_detached("HEAD", &intake)
        .expect("intake worktree");
    let config_path = temp.path().join(".codex/config.toml");
    let project_key = gwt_skills::register_codex_managed_project_trust(&intake, &config_path)
        .expect("seed exact Host Codex trust from an earlier Host launch")
        .project_path
        .to_string_lossy()
        .into_owned();
    let mut host_config: toml::Value =
        toml::from_str(&fs::read_to_string(&config_path).expect("read seeded config"))
            .expect("parse seeded config");
    host_config.as_table_mut().expect("config table").insert(
        "model".to_string(),
        toml::Value::String("user-owned".to_string()),
    );
    fs::write(
        &config_path,
        toml::to_string_pretty(&host_config).expect("render host config"),
    )
    .expect("write host Codex sentinel");

    let mut session = sample_active_agent_session("tab-1", "tab-1::intake");
    session.branch_name = String::new();
    session.worktree_path = intake.clone();
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    // This cleanup contract is independent of the PTY backend: a final Docker
    // session must still remove only its exact stale Host trust record.
    gwt::project_runtime::workspace::finalize_ephemeral_worktree_for_project(Some(&repo), &session);

    assert!(!intake.exists(), "clean Docker intake worktree is removed");
    assert_eq!(
        codex_project_trust_level(&config_path, &project_key),
        None,
        "deleting the Host path must remove exact stale Host trust even when the final session used Docker"
    );
    let remaining: toml::Value =
        toml::from_str(&fs::read_to_string(config_path).expect("read host Codex sentinel"))
            .expect("parse remaining Host config");
    assert_eq!(
        remaining["model"].as_str(),
        Some("user-owned"),
        "Docker-local lifecycle must not disturb unrelated Host Codex state"
    );
}

// SPEC-3214 (codex #3235 review): a NORMAL branch worktree that a user happens
// to name `.intake-*` must NOT be misclassified as an ephemeral intake session
// — it keeps its worktree and its Paused-Work behavior. Classification requires
// the worktree to be branchless (detached), not just `.intake-*`-named.
#[test]
fn branch_worktree_named_intake_is_not_treated_as_ephemeral() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    // A real BRANCH worktree that merely happens to be named `.intake-real`.
    let branch_wt = temp.path().join(".intake-real");
    gwt_git::WorktreeManager::new(&repo)
        .create_from_base("HEAD", "feature/real", &branch_wt)
        .expect("branch worktree");
    assert!(branch_wt.exists());

    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (owner, pane) = managed_owner_session_at(&repo, &branch_wt, "feature/real");
    super::pane_control_tests::control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());

    assert!(
        branch_wt.exists(),
        "a real branch worktree named .intake-* must not be removed as ephemeral"
    );
    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .ok()
        .flatten();
    assert!(
        works.is_some_and(|projection| !projection.work_items.is_empty()),
        "a real branch session still records a Paused Work"
    );
}

// #3065: a stopped session's Pause record must not inherit the repo-shared
// projection's owner/title — those belong to whatever Work last wrote the
// projection. Owner/summary come from the session's own Work item (matched
// by branch container); the title fallback is the matched item's title.
#[test]
fn paused_work_does_not_inherit_shared_projection_owner() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");

    let _gwt_home = ScopedGwtHome::set(temp.path());
    init_repo_with_initial_commit(&repo);
    let worktree = repo.join("work/paused");
    gwt_git::WorktreeManager::new(&repo)
        .create_from_base("HEAD", "work/paused", &worktree)
        .unwrap();
    let (owner, pane) = managed_owner_session_at(&repo, &worktree, "work/paused");
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let paused_work_id = format!("work-session-{session_id}");

    // Shared projection poisoned with a foreign Work's identity. The agent
    // summary carries no title of its own, so the old code fell back to the
    // shared title and copied the shared owner verbatim.
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.id = "work-foreign-99999999".to_string();
    projection.title = "gwt-manage-pr".to_string();
    projection.owner = Some("SPEC-2359".to_string());
    projection.summary = Some("foreign summary".to_string());
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test(&session_id, Some("work-paused"));
        agent.window_id = Some(pane.id.clone());
        agent.branch = Some("work/paused".to_string());
        agent.title_summary = None;
        agent.current_focus = None;
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    // The session's own Work item with its own identity.
    let now = chrono::Utc::now();
    let work_id =
        gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/paused"), None)
            .expect("canonical id");
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    event.title = Some("own work title".to_string());
    event.owner = Some("Issue #7".to_string());
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/paused".to_string()),
            worktree_path: Some(repo.join("work/paused")),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    record_legacy_workspace_work_event_for_test(&repo, event).expect("record work event");

    super::pane_control_tests::control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());

    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load works")
        .expect("works projection");
    let paused = works
        .work_items
        .iter()
        .find(|item| item.id == paused_work_id)
        .expect("paused session work item");
    assert_eq!(
        paused.owner.as_deref(),
        Some("Issue #7"),
        "pause must carry the session's own work owner, not the shared projection's"
    );
    assert_eq!(
        paused.title, "own work title",
        "pause title falls back to the session's own work item, not the shared projection"
    );
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): closing a Paused Work with
/// `close_kind = "done"` records a terminal Done close and removes the Work
/// from the active Work surface. No live agent owns the Work, so the close
/// is not blocked.
#[test]
fn app_runtime_close_work_done_removes_paused_work_from_active_surface() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    init_repo_with_initial_commit(&repo);
    let worktree = repo.join("work/done");
    gwt_git::WorktreeManager::new(&repo)
        .create_from_base("HEAD", "work/done", &worktree)
        .unwrap();
    let (owner, pane) = managed_owner_session_at(&repo, &worktree, "work/done");
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let stopped_work_id = format!("work-session-{session_id}");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test(&session_id, Some("work-done"));
        agent.window_id = Some(pane.id.clone());
        agent.branch = Some("work/done".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    super::pane_control_tests::control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());
    let paused_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("paused projection view");
    assert_eq!(paused_view.active_works.len(), 1);
    assert_eq!(paused_view.active_works[0].lifecycle_state, "paused");

    // Close (Done): the Work leaves the active surface.
    let events = runtime.close_work(&runtime.test_context(), stopped_work_id.as_str(), "done");
    assert!(
        events.is_empty(),
        "close_work schedules the refreshed projection off the event-loop path"
    );

    let closed_view = wait_for_active_work_projection(&mut runtime);
    assert!(
        closed_view
            .active_works
            .iter()
            .all(|work| work.id != stopped_work_id.as_str()),
        "Done-closed Work must not appear in active_works"
    );

    // The retained work history records the Done terminal close.
    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load work items");
    let item = works
        .work_items
        .iter()
        .find(|item| item.id == stopped_work_id.as_str())
        .expect("work item exists");
    assert_eq!(
        item.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Done
    );
    assert!(!item.discarded);
    assert!(item.is_terminal());
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): closing a Paused Work with
/// `close_kind = "discarded"` records a terminal Discard close (distinct from
/// Done) and removes the Work from the active surface.
#[test]
fn app_runtime_close_work_discarded_marks_terminal_and_removes_from_surface() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    init_repo_with_initial_commit(&repo);
    let worktree = repo.join("work/discard");
    gwt_git::WorktreeManager::new(&repo)
        .create_from_base("HEAD", "work/discard", &worktree)
        .unwrap();
    let (owner, pane) = managed_owner_session_at(&repo, &worktree, "work/discard");
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let stopped_work_id = format!("work-session-{session_id}");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test(&session_id, Some("work-discard"));
        agent.window_id = Some(pane.id.clone());
        agent.branch = Some("work/discard".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    super::pane_control_tests::control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());
    let events = runtime.close_work(
        &runtime.test_context(),
        stopped_work_id.as_str(),
        "discarded",
    );
    assert!(events.is_empty());

    let closed_view = wait_for_active_work_projection(&mut runtime);
    assert!(
        closed_view
            .active_works
            .iter()
            .all(|work| work.id != stopped_work_id.as_str()),
        "Discarded Work must not appear in active_works"
    );

    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load work items");
    let item = works
        .work_items
        .iter()
        .find(|item| item.id == stopped_work_id.as_str())
        .expect("work item exists");
    assert!(item.discarded, "Discard close must mark the Work discarded");
    assert_ne!(
        item.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
        "Discard is distinct from Done"
    );
    assert!(item.is_terminal());
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): a close request for a Work whose
/// owning agent session is still live must be blocked. The worktree is not
/// removed and the Work stays Active on the surface — the agent must be
/// stopped first.
#[test]
fn app_runtime_close_work_blocks_when_owning_agent_is_live() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    let worktree_path = repo.join("work/live");
    gwt_git::WorktreeManager::new(&repo)
        .create_from_base("HEAD", "work/live", &worktree_path)
        .unwrap();
    seed_owner_fixture_roster(&repo, 4);
    let owner = owner_fixture_block_on(production_owner_fixture(&repo));
    let id = owner.pane_id("live-work");
    let launched = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::LaunchAgent {
            id: id.clone(),
            operation_id: "live-work-close".into(),
            tab_id: "tab-1".into(),
            config: Box::new(managed_owner_config_at(&repo, &worktree_path, "work/live")),
            metadata: Box::new(gwt::project_runtime::launch_payload::OwnerLaunchMetadata {
                workspace_resume_context: Some(WorkspaceResumeContext {
                    title: Some("Live Work".into()),
                    owner: None,
                    summary: None,
                    next_action: None,
                }),
                ..Default::default()
            }),
            cols: 80,
            rows: 24,
        },
    ))
    .expect("actual owner publishes the live assigned Work");
    let pane = owner_fixture_block_on(owner.probe(&id, launched["incarnation"].as_u64().unwrap()))
        .unwrap();
    let source = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .unwrap()
        .unwrap();
    let work_id = source
        .latest_agent_for_session(&pane.session.as_ref().unwrap().id)
        .and_then(|agent| agent.workspace_id.clone())
        .expect("canonical Work assigned to the actual live Session");
    let mut runtime = managed_project_owner_projection(temp.path(), &repo, &owner, &[&pane]);
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let events = runtime.close_work(&runtime.test_context(), &work_id, "done");
    assert!(
        events.is_empty(),
        "a blocked close must not broadcast a projection change"
    );
    tasks.lock().unwrap().remove(0)();
    assert!(
        worktree_path.exists(),
        "blocked close must never remove the live worktree"
    );
    assert!(pane.pty.try_wait().unwrap().is_none());
    let live_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("live projection view");
    let work = live_view
        .active_works
        .iter()
        .find(|work| work.id == work_id)
        .unwrap_or_else(|| {
            panic!(
                "live Work {work_id} still present: {:?}",
                live_view
                    .active_works
                    .iter()
                    .map(|work| (&work.id, &work.lifecycle_state, &work.works))
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(work.lifecycle_state, "active");
    let works =
        gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo).unwrap();
    assert!(
        works
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .is_none_or(|item| !item.is_terminal()),
        "a blocked close must not record a terminal event"
    );
    super::pane_control_tests::control_close(&owner, &pane);
}

/// SPEC-2359 W-21 (FR-463): Work close records lifecycle history only. The
/// actual worktree and branch remain until the independent cleanup transport
/// removes them.
#[test]
fn app_runtime_close_work_retains_worktree_and_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");

    // Initialize a real git repo with an initial commit.
    let run_git = |args: &[&str], cwd: &Path| {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["init"], &repo);
    run_git(&["config", "user.email", "test@example.com"], &repo);
    run_git(&["config", "user.name", "Test User"], &repo);
    run_git(&["commit", "--allow-empty", "-m", "init"], &repo);

    // Create a real worktree on a dedicated branch.
    let manager = gwt_git::WorktreeManager::new(&repo);
    let base = if crate::runtime_support::local_branch_exists(&repo, "main").unwrap_or(false) {
        "main"
    } else {
        "master"
    };
    let worktree_path = temp.path().join("work-cleanup");
    manager
        .create_from_base(base, "work/cleanup", &worktree_path)
        .expect("create worktree");
    assert!(worktree_path.exists());

    let _gwt_home = ScopedGwtHome::set(temp.path());
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&repo),
    )
    .unwrap();

    // A saved (empty) Workspace projection so the close broadcast can build
    // the active-work projection view for the tab.
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    // Persist a Paused Work whose execution container points at the worktree.
    let now = chrono::Utc::now();
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        "work-session-session-cleanup",
        Some("Cleanup work"),
        None,
        None,
        &[],
        Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/cleanup".to_string()),
                worktree_path: Some(worktree_path.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        ),
        Some("session-cleanup"),
        now,
    )
    .expect("record paused work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    // No live agent session: close the Work without cleaning its materialization.
    let events = runtime.close_work(
        &runtime.test_context(),
        "work-session-session-cleanup",
        "done",
    );
    assert!(events.is_empty());
    let _closed_projection = wait_for_active_work_projection(&mut runtime);

    assert!(
        worktree_path.exists(),
        "Work close must retain the worktree directory"
    );
    assert!(
        crate::runtime_support::local_branch_exists(&repo, "work/cleanup").unwrap_or(false),
        "worktree-only cleanup must retain the branch (branch / PR are preserved)"
    );
    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load Work history");
    let closed = works
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-cleanup")
        .expect("closed Work remains in history");
    assert!(closed.is_terminal());
    owner_fixture_block_on(owner.shutdown());
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): resuming a paused Work (a live
/// agent session for the same `session_id` returns) must surface a single
/// Active row — the live grouping wins and no duplicate Paused row is
/// emitted from the retained work history.
#[test]
fn app_runtime_active_work_projection_resumed_paused_work_is_single_active_row() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let worktree = repo.join("work/resume");
    fs::create_dir_all(&worktree).expect("create worktree path");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-resume", Some("work-resume"));
        agent.window_id = Some("tab-1::agent-resume".to_string());
        agent.branch = Some("work/resume".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let build_session = |runtime: &mut AppRuntime, worktree_path: PathBuf| {
        let mut session = sample_active_agent_session("tab-1", "tab-1::agent-resume");
        session.session_id = "session-resume".to_string();
        session.branch_name = "work/resume".to_string();
        session.worktree_path = worktree_path;
        session.window_id = "tab-1::agent-resume".to_string();
        runtime
            .active_agent_sessions
            .insert("tab-1::agent-resume".to_string(), session);
    };
    build_session(&mut runtime, worktree.clone());

    // This is the projection's immutable same-Session input contract. Physical
    // restore/cleanup is verified separately by actual owner launch tests.
    let stopped = runtime
        .active_agent_sessions
        .remove("tab-1::agent-resume")
        .unwrap();
    gwt::project_runtime::workspace::paused_work_record_task(&repo, &stopped, Utc::now())
        .expect("canonical domain Paused Work record")();
    let paused_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("paused projection view");
    assert_eq!(paused_view.active_works.len(), 1);
    assert_eq!(paused_view.active_works[0].lifecycle_state, "paused");

    // Resume: the same session returns to active_agent_sessions.
    let alternate_worktree_path = worktree.join("..").join("resume");
    assert_ne!(worktree, alternate_worktree_path);
    assert!(same_worktree_path(&worktree, &alternate_worktree_path));
    build_session(&mut runtime, alternate_worktree_path);
    let resumed_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("resumed projection view");
    assert_eq!(
        resumed_view.active_works.len(),
        1,
        "resumed Work must dedupe to a single Active row (no Paused duplicate)"
    );
    assert_eq!(
        resumed_view.active_works[0].id,
        "work-session-session-resume"
    );
    assert_eq!(resumed_view.active_works[0].lifecycle_state, "active");
    assert_eq!(
        resumed_view.active_works[0].works.len(),
        1,
        "one resumed Session must stay one child Work across equivalent worktree paths"
    );
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): a live Work and an unrelated
/// paused Work coexist as two rows — the live one Active, the retained one
/// Paused — without the merge collapsing or dropping either.
#[test]
fn app_runtime_active_work_projection_merges_live_and_paused_work_rows() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    let live_worktree = repo.join("work/live");
    let stopped_worktree = repo.join("work/stop");
    let manager = gwt_git::WorktreeManager::new(&repo);
    manager
        .create_from_base("HEAD", "work/live", &live_worktree)
        .unwrap();
    manager
        .create_from_base("HEAD", "work/stop", &stopped_worktree)
        .unwrap();
    let (owner, live) = managed_owner_session_at(&repo, &live_worktree, "work/live");
    let stopped = super::pane_control_tests::control_owner_launch(
        &owner,
        "stopped",
        managed_owner_config_at(&repo, &stopped_worktree, "work/stop"),
    );
    super::pane_control_tests::control_close(&owner, &stopped);
    assert!(stopped.pty.try_wait().unwrap().is_some());
    assert!(live.pty.try_wait().unwrap().is_none());
    let runtime = managed_project_owner_projection(temp.path(), &repo, &owner, &[&live]);
    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    assert_eq!(view.active_works.len(), 2, "live + paused Work rows");
    let live_work = view
        .active_works
        .iter()
        .find(|work| work.id == format!("work-session-{}", live.session.as_ref().unwrap().id))
        .expect("live Work row");
    assert_eq!(live_work.lifecycle_state, "active");
    let paused = view
        .active_works
        .iter()
        .find(|work| work.id == format!("work-session-{}", stopped.session.as_ref().unwrap().id))
        .expect("paused Work row");
    assert_eq!(paused.lifecycle_state, "paused");
    assert_eq!(paused.active_agents, 0);
    super::pane_control_tests::control_close(&owner, &live);
}

/// Issue #3213 regression (PR #3205 orphaned): a stray agent ref that shares a
/// session id with ANOTHER branch's Work must not swallow that Work's row.
/// Reproduces the affected project's works.json: the issue-3184 item carried a
/// mis-attributed (empty-identity) ref to issue-3197's session, and the
/// issue-3197 row — the session's legitimate owner — vanished from the
/// Workspace list with no remaining surface to resume it from.
#[test]
fn app_runtime_paused_work_row_survives_stray_shared_session_on_other_branch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    let works = vec![
        history_work_view(
            "work-work-issue-3184-9431c779",
            "work/issue-3184",
            "/home/user/gwt/work/issue-3184",
            vec![
                history_agent_ref_view(
                    "810665c4-2e03-46b3-879b-7eec0064d038",
                    Some("Claude Code"),
                    "2026-06-29T07:46:15Z",
                ),
                // The stray ref: recorded under issue-3184 with an empty
                // identity, but the session belongs to issue-3197.
                history_agent_ref_view(
                    "0fe1b919-09dd-47e5-8976-76f4478aa907",
                    None,
                    "2026-06-29T08:24:29Z",
                ),
            ],
        ),
        history_work_view(
            "work-work-issue-3197-00504508",
            "work/issue-3197",
            "/home/user/gwt/work/issue-3197",
            vec![history_agent_ref_view(
                "0fe1b919-09dd-47e5-8976-76f4478aa907",
                Some("Claude Code"),
                "2026-06-29T07:45:56Z",
            )],
        ),
    ];

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        works,
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "both branch rows must surface despite the shared session id"
    );
    let issue_3197 = view
        .active_works
        .iter()
        .find(|work| work.id == "work-work-issue-3197-00504508")
        .expect("work/issue-3197 row must not be swallowed by the stray session ref");
    assert_eq!(issue_3197.branch.as_deref(), Some("work/issue-3197"));
    assert_eq!(issue_3197.lifecycle_state, "paused");
}

/// SPEC-2359 FR-471: a shared Session id cannot override a conflicting
/// worktree identity even when the branch identity agrees.
#[test]
fn app_runtime_shared_session_with_conflicting_worktree_keeps_both_works() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.git_details = Some(git_details_for_active_work_test(
        "work/shared",
        "/home/user/gwt/work/paused",
    ));
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("shared-session", Some("work-live"));
        agent.branch = Some("work/shared".to_string());
        agent.worktree_path = Some(PathBuf::from("/home/user/gwt/work/live"));
        agent
    });
    let mut paused = history_work_view(
        "legacy-work-paused",
        "work/shared",
        "/home/user/gwt/work/paused",
        vec![history_agent_ref_view(
            "shared-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    paused.title = "Paused worktree-conflict history".to_string();
    paused.summary = Some("Paused worktree-conflict summary".to_string());
    paused.owner = Some("Issue #471".to_string());
    paused.execution_containers[0].pr_number = Some(471);
    paused.execution_containers[0].pr_url = Some("https://example.test/pr/471".to_string());
    paused.board_refs = vec!["paused-board-ref".to_string()];

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![paused],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "a worktree conflict must prevent shared-Session deduplication"
    );
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-shared-session")
        .expect("live Work");
    assert_eq!(live.title, "Board audience follow-up");
    assert_eq!(live.summary, None);
    assert_eq!(live.owner, None);
    assert_eq!(live.pr_number, None);
    assert!(live.board_refs.is_empty());
}

/// SPEC-2359 FR-471: a shared Session id cannot override a conflicting
/// branch identity even when the worktree identity agrees.
#[test]
fn app_runtime_shared_session_with_conflicting_branch_keeps_both_works() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let shared_worktree = "/home/user/gwt/work/shared";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.git_details = Some(git_details_for_active_work_test(
        "work/paused",
        shared_worktree,
    ));
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("shared-session", Some("work-live"));
        agent.branch = Some("work/live".to_string());
        agent.worktree_path = Some(PathBuf::from(shared_worktree));
        agent
    });
    let mut paused = history_work_view(
        "legacy-work-paused",
        "work/paused",
        shared_worktree,
        vec![history_agent_ref_view(
            "shared-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    paused.title = "Paused branch-conflict history".to_string();
    paused.summary = Some("Paused branch-conflict summary".to_string());
    paused.owner = Some("Issue #472".to_string());
    paused.execution_containers[0].pr_number = Some(472);
    paused.execution_containers[0].pr_url = Some("https://example.test/pr/472".to_string());
    paused.board_refs = vec!["paused-board-ref".to_string()];

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![paused],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "a branch conflict must prevent shared-Session deduplication"
    );
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-shared-session")
        .expect("live Work");
    assert_eq!(live.title, "Board audience follow-up");
    assert_eq!(live.summary, None);
    assert_eq!(live.owner, None);
    assert_eq!(live.pr_number, None);
    assert!(live.board_refs.is_empty());
}

/// SPEC-2359 FR-471: projection fallback must participate in history matching
/// when the live Agent omits branch identity, so foreign history metadata does
/// not leak through a matching worktree.
#[test]
fn app_runtime_projection_branch_fallback_blocks_foreign_history_metadata() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let shared_worktree = "/home/user/gwt/work/shared";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "Live projection title".to_string();
    projection.git_details = Some(git_details_for_active_work_test(
        "work/live",
        shared_worktree,
    ));
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("shared-session", Some("work-live"));
        agent.branch = None;
        agent.worktree_path = Some(PathBuf::from(shared_worktree));
        agent
    });
    let mut foreign = history_work_view(
        "legacy-work-foreign",
        "work/foreign",
        shared_worktree,
        vec![history_agent_ref_view(
            "shared-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    foreign.title = "Foreign history title".to_string();
    foreign.summary = Some("Foreign history summary".to_string());

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![foreign],
        None,
    );

    assert_eq!(view.active_works.len(), 2);
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-shared-session")
        .expect("live Work");
    assert_eq!(live.branch.as_deref(), Some("work/live"));
    assert_eq!(live.title, "Live projection title");
    assert_eq!(live.summary, None);
}

/// SPEC-2359 FR-471: projection fallback must participate in history matching
/// when the live Agent omits worktree identity, so foreign history metadata
/// does not leak through a matching branch.
#[test]
fn app_runtime_projection_worktree_fallback_blocks_foreign_history_metadata() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let live_worktree = "/home/user/gwt/work/live";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "Live projection title".to_string();
    projection.git_details = Some(git_details_for_active_work_test(
        "work/shared",
        live_worktree,
    ));
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("shared-session", Some("work-live"));
        agent.branch = Some("work/shared".to_string());
        agent.worktree_path = None;
        agent
    });
    let mut foreign = history_work_view(
        "legacy-work-foreign",
        "work/shared",
        "/home/user/gwt/work/foreign",
        vec![history_agent_ref_view(
            "shared-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    foreign.title = "Foreign history title".to_string();
    foreign.summary = Some("Foreign history summary".to_string());

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![foreign],
        None,
    );

    assert_eq!(view.active_works.len(), 2);
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-shared-session")
        .expect("live Work");
    assert_eq!(live.worktree_path.as_deref(), Some(live_worktree));
    assert_eq!(live.title, "Live projection title");
    assert_eq!(live.summary, None);
}

/// SPEC-2359 FR-470/FR-471: branch/worktree can group a parent Workspace but
/// cannot assign a legacy history record to a live Work when neither exact
/// Work id nor a non-empty shared Session identifies that launch.
#[test]
fn app_runtime_sessionless_live_work_does_not_claim_legacy_history_by_git_identity() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let shared_worktree = "/home/user/gwt/work/shared";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("", Some("work-live"));
        agent.branch = Some("work/shared".to_string());
        agent.worktree_path = Some(PathBuf::from(shared_worktree));
        agent
    });
    let mut legacy = history_work_view(
        "legacy-work-paused",
        "work/shared",
        shared_worktree,
        Vec::new(),
    );
    legacy.title = "Legacy history metadata".to_string();
    legacy.summary = Some("Legacy history summary".to_string());

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![legacy],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "git identity alone must not collapse or assign launch-scoped Work metadata"
    );
    let live = view
        .active_works
        .iter()
        .find(|work| work.id != "legacy-work-paused")
        .expect("sessionless live Work");
    assert_eq!(live.title, "Board audience follow-up");
    assert_eq!(live.summary, None);
}

/// SPEC-2359 US-85 / SC-313: Work identity is launch-scoped even when two
/// stopped launches share one branch/worktree Workspace. They must survive
/// the history projection as separate child Works so each keeps its Session.
#[test]
fn app_runtime_paused_works_on_same_branch_remain_distinct_children() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);

    let mut older = history_work_view(
        "work-session-older",
        "work/shared",
        "/home/user/gwt/work/shared",
        vec![history_agent_ref_view(
            "older-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    older.title = "Older Work".to_string();
    older.updated_at = "2026-07-12T01:00:00Z".to_string();
    older.agents[0].sessions = vec![gwt::WorkspaceHistorySessionView {
        agent_session_id: "older-conversation".to_string(),
        started_at: "2026-07-12T01:00:00Z".to_string(),
        is_active: true,
        resumable: true,
    }];

    let mut newer = history_work_view(
        "work-session-newer",
        "work/shared",
        "/home/user/gwt/work/shared",
        vec![history_agent_ref_view(
            "newer-session",
            Some("Codex"),
            "2026-07-13T01:00:00Z",
        )],
    );
    newer.title = "Newer Work".to_string();
    newer.updated_at = "2026-07-13T01:00:00Z".to_string();
    newer.agents[0].sessions = vec![gwt::WorkspaceHistorySessionView {
        agent_session_id: "newer-conversation".to_string(),
        started_at: "2026-07-13T01:00:00Z".to_string(),
        is_active: true,
        resumable: true,
    }];

    let mut view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![older, newer],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "branch equality groups a Workspace later; it must not erase a distinct Work"
    );

    super::super::assign_and_merge_workspace_groups(&mut view.active_works, &repo, &HashMap::new());
    super::super::attach_registry_sessions_to_active_works(
        &mut view.active_works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert_eq!(view.active_works.len(), 1, "one branch is one Workspace");
    let workspace = &view.active_works[0];
    assert_eq!(workspace.works.len(), 2, "both launch-scoped Works remain");
    for (work_id, session_id, conversation_id) in [
        ("work-session-older", "older-session", "older-conversation"),
        ("work-session-newer", "newer-session", "newer-conversation"),
    ] {
        let child = workspace
            .works
            .iter()
            .find(|child| child.id == work_id)
            .expect("child Work");
        assert_eq!(child.agents.len(), 1);
        assert_eq!(child.agents[0].session_id, session_id);
        assert_eq!(child.agents[0].sessions.len(), 1);
        assert_eq!(
            child.agents[0].sessions[0].agent_session_id,
            conversation_id
        );
    }
}

/// SPEC-2359 FR-470/FR-471: branch/worktree identify the parent Workspace,
/// not one launch-scoped Work. A live Session must not hide an older Paused
/// Work on the same execution container when their Session identities differ.
#[test]
fn app_runtime_live_work_keeps_distinct_paused_work_on_same_branch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let shared_worktree = "/home/user/gwt/work/shared";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("live-session", Some("work-live"));
        agent.branch = Some("work/shared".to_string());
        agent.worktree_path = Some(std::path::PathBuf::from(shared_worktree));
        agent
    });

    let mut paused = history_work_view(
        "work-session-paused-session",
        "work/shared",
        shared_worktree,
        vec![history_agent_ref_view(
            "paused-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    paused.title = "Paused Work".to_string();
    paused.summary = Some("Paused Work summary".to_string());
    paused.owner = Some("Issue #470".to_string());
    paused.execution_containers[0].pr_number = Some(470);
    paused.execution_containers[0].pr_url = Some("https://example.test/pr/470".to_string());
    paused.board_refs = vec!["paused-work-board-ref".to_string()];
    paused.agents[0].sessions = vec![gwt::WorkspaceHistorySessionView {
        agent_session_id: "paused-conversation".to_string(),
        started_at: "2026-07-12T01:00:00Z".to_string(),
        is_active: true,
        resumable: true,
    }];

    let mut view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![paused],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "same Workspace identity must not collapse different live/paused Sessions"
    );
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-live-session")
        .expect("live child Work");
    assert_eq!(live.title, "Board audience follow-up");
    assert_eq!(live.summary, None);
    assert_eq!(live.owner, None);
    assert_eq!(live.pr_number, None);
    assert!(live.board_refs.is_empty());

    super::super::assign_and_merge_workspace_groups(&mut view.active_works, &repo, &HashMap::new());
    super::super::attach_registry_sessions_to_active_works(
        &mut view.active_works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert_eq!(view.active_works.len(), 1, "one branch is one Workspace");
    let workspace = &view.active_works[0];
    assert_eq!(workspace.works.len(), 2, "both Session-owned Works remain");
    let paused = workspace
        .works
        .iter()
        .find(|work| work.id == "work-session-paused-session")
        .expect("Paused child Work");
    assert_eq!(paused.lifecycle_state, "paused");
    assert_eq!(paused.agents.len(), 1);
    assert_eq!(paused.agents[0].session_id, "paused-session");
}

/// SPEC-2359 FR-471: a legacy history id and a live Session-derived Work id
/// still represent one resumed Work when their worktree paths are equivalent
/// filesystem paths with different lexical spellings.
#[test]
fn app_runtime_resumed_work_dedupes_missing_lexically_equivalent_worktree_paths() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let worktree = repo.join("work/resume");
    let alternate_worktree_path = worktree.join("..").join("resume");
    assert_ne!(worktree, alternate_worktree_path);
    assert!(
        !worktree.exists(),
        "projection path identity must not depend on filesystem existence"
    );

    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-resume-alias", Some("work-live"));
        agent.branch = None;
        agent.worktree_path = Some(alternate_worktree_path);
        agent
    });

    let mut paused = history_work_view(
        "legacy-work-resume-alias",
        "",
        worktree.to_string_lossy().as_ref(),
        vec![history_agent_ref_view(
            "session-resume-alias",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    paused.execution_containers[0].branch = None;
    paused.title = "Resumed history metadata".to_string();
    paused.summary = Some("Resumed history summary".to_string());
    paused.owner = Some("SPEC-2359".to_string());
    paused.execution_containers[0].pr_number = Some(2359);
    paused.execution_containers[0].pr_url = Some("https://example.test/pr/2359".to_string());
    paused.board_refs = vec!["resumed-board-ref".to_string()];

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![paused],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        1,
        "shared Session must dedupe equivalent worktree path spellings"
    );
    assert_eq!(view.active_works[0].id, "work-session-session-resume-alias");
    assert_eq!(view.active_works[0].title, "Resumed history metadata");
    assert_eq!(
        view.active_works[0].summary.as_deref(),
        Some("Resumed history summary")
    );
    assert_eq!(view.active_works[0].owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(view.active_works[0].pr_number, Some(2359));
    assert_eq!(
        view.active_works[0].board_refs,
        vec!["resumed-board-ref".to_string()]
    );
}

/// FR-348 compatibility guard: Work identity is agent-session-derived, so a
/// legacy and canonical history row for the same Session remain one Work even
/// after distinct same-Workspace Sessions stop collapsing by git identity.
#[test]
fn app_runtime_duplicate_paused_history_for_same_session_stays_one_work() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    let works = vec![
        history_work_view(
            "legacy-work-id",
            "work/shared",
            "/home/user/gwt/work/shared",
            vec![history_agent_ref_view(
                "shared-session",
                Some("Codex"),
                "2026-07-12T01:00:00Z",
            )],
        ),
        history_work_view(
            "work-session-shared-session",
            "work/shared",
            "/home/user/gwt/work/shared",
            vec![history_agent_ref_view(
                "shared-session",
                Some("Codex"),
                "2026-07-13T01:00:00Z",
            )],
        ),
    ];

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        works,
        None,
    );

    assert_eq!(
        view.active_works.len(),
        1,
        "the same launch Session must not become two Paused Works"
    );
    assert_eq!(view.active_works[0].agents[0].session_id, "shared-session");
}

/// SPEC-3170 AS-12.12 / FR-076: rebuilding the Workspace projection must not
/// compare every paused Work with every row already appended. A large retained
/// history is common in long-lived repositories, so unrelated Session ids
/// should bypass git-identity conflict checks entirely.
#[test]
fn app_runtime_paused_history_dedupe_scales_with_session_matches() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    let works = (0..256)
        .map(|index| {
            history_work_view(
                &format!("work-history-{index}"),
                &format!("work/history-{index}"),
                &format!("/home/user/gwt/work/history-{index}"),
                vec![history_agent_ref_view(
                    &format!("session-history-{index}"),
                    Some("Codex"),
                    "2026-07-12T01:00:00Z",
                )],
            )
        })
        .collect::<Vec<_>>();

    super::super::workspace_views::reset_history_git_identity_conflict_checks();
    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        works,
        None,
    );
    let conflict_checks = super::super::workspace_views::history_git_identity_conflict_checks();

    assert_eq!(view.active_works.len(), 256);
    assert!(
        conflict_checks <= 256,
        "unrelated Sessions must stay linear; observed {conflict_checks} identity checks"
    );
}

/// FR-350 contract guard for the #3213 fix: a live Work synthesized without
/// git_details (no branch / worktree on the row) still dedupes the
/// session-sharing history item — the original purpose of the session-id
/// fallback in `active_work_already_present`.
#[test]
fn app_runtime_paused_work_dedups_by_session_when_live_row_has_no_git_identity() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-shared", Some("work-x"));
        agent.branch = None;
        agent.worktree_path = None;
        agent
    });
    // The history row carries a full git identity and the same session.
    let works = vec![history_work_view(
        "work-work-other-0000abcd",
        "work/other",
        "/home/user/gwt/work/other",
        vec![history_agent_ref_view(
            "session-shared",
            Some("Claude Code"),
            "2026-06-29T07:45:56Z",
        )],
    )];

    let view = super::super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        works,
        None,
    );

    assert_eq!(
        view.active_works.len(),
        1,
        "identity-less live row + session-sharing history must stay one row"
    );
}

#[test]
fn app_runtime_active_work_projection_resolves_branch_known_unassigned_agents_as_work() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.register_unassigned_agent({
        let mut agent = workspace_agent_summary_for_test("session-unassigned", None);
        agent.window_id = Some("tab-1::agent-unassigned".to_string());
        agent.branch = Some("work/unassigned-but-known".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-unassigned");
    session.session_id = "session-unassigned".to_string();
    session.branch_name = "work/unassigned-but-known".to_string();
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session);

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 1);
    assert_eq!(view.active_works.len(), 1);
    assert_eq!(view.active_works[0].agents.len(), 1);
    assert!(view.unassigned_agents.is_empty());
}

#[test]
fn app_runtime_open_active_work_launch_wizard_focuses_existing_agent_for_branch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.branch_name = "work/test".to_string();
    session.window_id = window_id.clone();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);

    let events = runtime.open_active_work_launch_wizard(
        &runtime.test_context(),
        "client-1",
        "work/test",
        None,
    );

    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .is_none());
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Project(_),
            event: BackendEvent::WorkspaceState { .. },
            ..
        }
    )));
}

#[test]
fn app_runtime_live_work_agent_lookup_ignores_stopped_or_error_windows() {
    for status in [WindowProcessStatus::Stopped, WindowProcessStatus::Error] {
        let temp = tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "agent-1",
            repo,
            WindowPreset::Agent,
            status,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        let mut session = sample_active_agent_session("tab-1", &window_id);
        session.branch_name = "work/test".to_string();
        session.window_id = window_id.clone();
        runtime
            .active_agent_sessions
            .insert(window_id.clone(), session);

        assert_eq!(
            runtime.live_agent_window_for_work("tab-1", Some("work/test"), None),
            None,
            "{status:?} windows must not block a later launch"
        );
    }
}

#[test]
fn app_runtime_active_work_projection_promotes_branch_known_unassigned_agents_to_active_work() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.register_unassigned_agent({
        let mut agent = workspace_agent_summary_for_test("session-unassigned", None);
        agent.window_id = Some("tab-1::agent-unassigned".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-unassigned");
    session.session_id = "session-unassigned".to_string();
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session);

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 1);
    assert_eq!(view.active_works.len(), 1);
    assert_eq!(view.active_works[0].agents.len(), 1);
    assert!(view.unassigned_agents.is_empty());
}

#[test]
fn app_runtime_launch_failure_log_redacts_sensitive_error_values() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let events = capture_tracing_events(|| {
        let (owner, caller, _) = super::execution_genesis_tests::genesis_pre_pty_failure_fixture_with_command(temp.path(),
            "/definitely/missing/pre-pty-provider OPENAI_API_KEY=sk-test --api-key sk-other GWT_HOOK_TOKEN=hook-secret --token plain-token");
        pane_control_tests::control_close(&owner, &caller);
    });
    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("launch_complete")
        })
        .expect("actual redacted worker failure log");
    let error = event.fields.get("error").unwrap();
    for secret in ["sk-test", "sk-other", "hook-secret", "plain-token"] {
        assert!(
            !error.contains(secret),
            "sensitive launch values must not reach the owner log"
        );
    }
    for redacted in [
        "OPENAI_API_KEY=[REDACTED]",
        "--api-key [REDACTED]",
        "GWT_HOOK_TOKEN=[REDACTED]",
        "--token [REDACTED]",
    ] {
        assert!(error.contains(redacted), "{error}");
    }
}

#[test]
fn launch_failure_retains_structured_retry_metadata_in_terminal_status() {
    use gwt::project_runtime::launch_payload::AgentLaunchError;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (owner, caller, captured) =
        super::execution_genesis_tests::genesis_pre_pty_failure_fixture(temp.path());
    let source = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", captured.session_id)),
    )
    .unwrap();
    let project = source.project_state_root.clone().unwrap();
    for (error, error_code, retryable) in [
        (
            AgentLaunchError::from("simulated launch failure"),
            "launch_failed",
            true,
        ),
        (
            AgentLaunchError::active_conflict("exact launch conflict"),
            "active_launch_conflict",
            true,
        ),
        (
            AgentLaunchError::authority_rejected("exact authority mismatch"),
            "active_launch_authority_rejected",
            false,
        ),
    ] {
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "failed",
            project.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Starting,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let context = runtime.test_context();
        let window_id = combined_window_id("tab-1", "failed");
        runtime.launch_wizard_cache.sessions();
        let generation = runtime.window_lifecycle_generations.lock().unwrap()[&window_id];
        let client_cell = Arc::new(Mutex::new(Some(Ok(owner.client()))));
        let cancel_ack = Arc::new(tokio::sync::Mutex::new(None));
        runtime.pending_launch_completions.insert(
            window_id.clone(),
            super::super::launch::PendingLaunchCompletion {
                generation,
                context: Some(context.clone()),
                current: Arc::new(AtomicBool::new(true)),
                operation_id: "actual-pre-pty-failure".into(),
                owner_pane_id: captured.id.clone(),
                client_cell: client_cell.clone(),
                cancel_ack: cancel_ack.clone(),
            },
        );
        // This is a read-model classification contract. Keep the actual owner
        // identity/Session fingerprint while exercising each canonical error's
        // presentation scalars; none of these observations authorizes input.
        let mut failure = captured.clone();
        failure.detail = error.detail;
        failure.error_code = error.error_code.into();
        failure.retryable = error.retryable;
        let events =
            runtime.handle_remote_owner_launch_complete(super::super::RemoteOwnerLaunchComplete {
                context,
                window_id: window_id.clone(),
                generation,
                owner_pane_id: captured.id.clone(),
                operation_id: "actual-pre-pty-failure".into(),
                client_cell,
                cancel_ack,
                restore_refusal: None,
                restore_feedback: None,
                result: Ok(super::super::RemoteOwnerLaunchOutcome::Failed {
                    failure,
                    session: Some(source.clone()),
                }),
            });
        let response = events
            .iter()
            .find(|event| {
                matches!(&event.event, BackendEvent::TerminalStatus {
            id, status: WindowProcessStatus::Error, .. } if id == &window_id)
            })
            .expect("actual owner failure presentation");
        let value = serde_json::to_value(&response.event).unwrap();
        assert_eq!(value["error_code"], error_code);
        assert_eq!(value["retryable"], retryable);
    }
    pane_control_tests::control_close(&owner, &caller);
}

#[test]
fn stale_runtime_events_cannot_mutate_same_session_window_successor() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let predecessor_incarnation = runtime.runtimes[&window_id].incarnation;
    close_test_owner_runtime(&mut runtime, &window_id);
    insert_test_pane_runtime(&mut runtime, &window_id);
    let successor_incarnation = runtime.runtimes[&window_id].incarnation;
    assert!(
        successor_incarnation > predecessor_incarnation,
        "runtime incarnations must increase monotonically within the process"
    );
    runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Running);
    runtime.window_output_bytes.insert(window_id.clone(), 41);
    runtime
        .window_details
        .insert(window_id.clone(), "successor ready".to_string());
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let output_events = runtime.handle_runtime_output_event(
        window_id.clone(),
        predecessor_incarnation,
        b"late predecessor output".to_vec(),
        1,
        None,
    );
    let status_events = runtime.handle_runtime_status_event(
        window_id.clone(),
        predecessor_incarnation,
        WindowProcessStatus::Stopped,
        Some("predecessor exited".to_string()),
        true,
    );

    assert!(output_events.is_empty());
    assert!(status_events.is_empty());
    assert_eq!(runtime.window_output_bytes.get(&window_id), Some(&41));
    assert_eq!(
        runtime.window_pty_statuses.get(&window_id),
        Some(&WindowProcessStatus::Running)
    );
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("successor ready")
    );
    assert_eq!(
        runtime
            .runtimes
            .get(&window_id)
            .map(|runtime| runtime.incarnation),
        Some(successor_incarnation)
    );
    let successor_screen = owner_runtime_probe(&runtime, &window_id)
        .pane
        .lock()
        .expect("successor pane")
        .screen()
        .contents();
    assert!(!successor_screen.contains("late predecessor output"));
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        runtime
            .active_agent_sessions
            .get(&window_id)
            .map(|session| session.session_id.as_str()),
        Some("session-1"),
        "the successor intentionally reuses the predecessor Session identity"
    );
    assert!(runtime.window_lookup.contains_key(&window_id));

    runtime.active_agent_sessions.remove(&window_id);
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn unconfirmed_runtime_error_cannot_publish_terminal_execution_proof() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-1", Some("unconfirmed-reader-error".to_string())));
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let identity = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "unconfirmed-reader-error",
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    let incarnation = runtime
        .runtimes
        .get(&window_id)
        .expect("live runtime")
        .incarnation;
    let runtime_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, &identity.session_id);
    gwt_agent::SessionRuntimeState::for_execution(
        gwt_agent::AgentStatus::Running,
        &identity,
        incarnation,
    )
    .save(&runtime_path)
    .expect("save exact Running runtime proof");
    let session_path = runtime
        .sessions_dir
        .join(format!("{}.toml", identity.session_id));
    let session_before = fs::read(&session_path).expect("read Running Session");

    runtime.handle_runtime_status_event(
        window_id.clone(),
        incarnation,
        WindowProcessStatus::Error,
        Some("PTY reader failed".to_string()),
        false,
    );

    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        fs::read(&session_path).expect("read Session after unconfirmed error"),
        session_before
    );
    let proof = gwt_agent::SessionRuntimeState::load(&runtime_path)
        .expect("load runtime proof after unconfirmed error");
    assert_eq!(proof.status, gwt_agent::AgentStatus::Running);
    assert_eq!(proof.execution_identity.as_ref(), Some(&identity));
    assert_eq!(proof.runtime_incarnation, Some(incarnation));

    runtime.active_agent_sessions.remove(&window_id);
    close_test_owner_runtime(&mut runtime, &window_id);
}

// Seed the predecessor through a real producing launch and exact owner Close.
// Only the benign Custom provider's native conversation is fixture metadata.
fn managed_bound_resume_source(
    temp: &Path,
) -> (
    Arc<gwt::project_runtime::test_support::ProjectOwnerHarness>,
    gwt_agent::Session,
    gwt_agent::LaunchConfig,
) {
    let project = temp.join("repo");
    let (owner, predecessor) = pane_control_tests::control_owner_agent(&project, "predecessor", 42);
    let session_id = predecessor.session.as_ref().unwrap().id.clone();
    pane_control_tests::control_close(&owner, &predecessor);
    assert!(predecessor.pty.tree_exit_confirmed());
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let mut source =
        gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))).unwrap();
    source.agent_session_id = Some("bound-resume-native-conversation".into());
    source.save(&sessions).unwrap();
    gwt_agent::save_stored_custom_agents_to_path(
        &gwt_core::paths::gwt_config_path(),
        &[gwt_agent::StoredCustomAgent::new(
            pane_control_tests::control_custom_agent(),
        )],
    )
    .unwrap();
    let config = gwt::session_launch::launch_config_from_persisted_session(&source);
    assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    assert_eq!(
        config.predecessor_session_id.as_deref(),
        Some(source.id.as_str())
    );
    (owner, source, config)
}

#[test]
fn production_bound_agent_launch_publishes_exact_runtime_and_natural_exit_retains_it() {
    use gwt::project_runtime::{
        events::{OwnerCommand, OwnerEvent},
        launch_payload::OwnerLaunchMetadata,
        test_support::OwnerLaunchStage,
    };
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _custom = ScopedEnvVar::unset(gwt_agent::DISABLE_GLOBAL_CUSTOM_AGENTS_ENV);
    let (owner, source, mut config) = managed_bound_resume_source(temp.path());
    let project = temp.path().join("repo");
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let runtime_path = gwt_agent::runtime_state_path(&sessions, &source.id);
    config.command = if cfg!(windows) {
        "powershell"
    } else {
        "/bin/sh"
    }
    .into();
    config.args = if cfg!(windows) {
        vec![
            "-NoProfile".into(),
            "-Command".into(),
            "$null = [Console]::ReadLine(); exit 0".into(),
        ]
    } else {
        vec!["-c".into(), "read ready; exit 0".into()]
    };
    let id = owner.pane_id("resumed");
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        let checkpoint = owner
            .pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall)
            .await
            .unwrap();
        let mut observed = owner.subscribe();
        let client = owner.client();
        let target = id.clone();
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::LaunchAgent {
                    id: target,
                    operation_id: "bound-natural-resume".into(),
                    tab_id: "bound-natural".into(),
                    config: Box::new(config),
                    metadata: Box::new(OwnerLaunchMetadata {
                        workspace_resume_context: Some(WorkspaceResumeContext {
                            title: Some("Restore the bound Work".into()),
                            owner: Some("SPEC-42".into()),
                            summary: None,
                            next_action: None,
                        }),
                        ..Default::default()
                    }),
                    cols: 80,
                    rows: 24,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual owner materialization checkpoint
        let prepared = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        let expected =
            gwt_agent::SessionExecutionIdentity::from_session(prepared.session.as_ref().unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(
            expected.session_id, source.id,
            "Resume retains the exact bound Session"
        );
        assert!(
            gwt_agent::active_launch_handshake_path(&sessions, &source.id).exists(),
            "the real Active Resume acquires its physical launch gate"
        );
        checkpoint.release();
        let receipt = launch.await.unwrap().unwrap();
        assert!(receipt.get("launch_failure").is_none(), "{receipt}");
        let incarnation = receipt["incarnation"].as_u64().unwrap();
        let pane = owner.probe(&id, incarnation).await.unwrap();
        let running = gwt_agent::SessionRuntimeState::load(&runtime_path).unwrap();
        assert_eq!(running.status, gwt_agent::AgentStatus::Running);
        assert_eq!(running.execution_identity.as_ref(), Some(&expected));
        assert_eq!(running.runtime_incarnation, Some(incarnation));
        assert!(running.child_pid.is_some_and(|pid| pid > 0));
        assert!(running.child_started_at.is_some_and(|birth| birth > 0));
        assert!(
            !gwt_agent::active_launch_handshake_path(&sessions, &source.id).exists(),
            "Running publication clears the exact child-spawned gate"
        );
        let works = gwt_core::workspace_projection::load_workspace_work_items(&project)
            .unwrap()
            .unwrap();
        let work = works
            .work_items
            .iter()
            .find(|work| {
                work.agents
                    .iter()
                    .any(|agent| agent.session_id == source.id)
            })
            .unwrap();
        assert_eq!(work.owner.as_deref(), Some("Issue #42"));
        assert_eq!(work.title, "Restore the bound Work");
        owner
            .command(OwnerCommand::Input {
                id: id.clone(),
                incarnation,
                data: b"finish\n".to_vec(),
            })
            .await
            .unwrap();
        // test-hygiene: allow-wall-clock-deadline hang guard for the real child's exit and exact owner finalizer ACK
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Ok(OwnerEvent::Closed {
                    id: ended,
                    incarnation: ended_incarnation,
                    error,
                }) = observed.recv().await
                {
                    if ended == id && ended_incarnation == incarnation {
                        assert!(error.is_none(), "{error:?}");
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert!(pane.pty.tree_exit_confirmed());
        let stopped = gwt_agent::SessionRuntimeState::load(&runtime_path).unwrap();
        assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
        assert_eq!(stopped.execution_identity.as_ref(), Some(&expected));
        assert_eq!(stopped.runtime_incarnation, Some(incarnation));
        assert_eq!(stopped.child_pid, running.child_pid);
        assert_eq!(stopped.child_started_at, running.child_started_at);
    });
    owner_fixture_block_on(Arc::try_unwrap(owner).ok().unwrap().shutdown());
}

#[test]
fn unbound_agent_pty_publishes_process_identity_for_session_observation() {
    use gwt::project_runtime::{
        events::OwnerCommand,
        test_support::{OwnerLaunchStage, ProjectOwnerHarness},
    };
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "-c",
            "user.name=Identity fixture",
            "-c",
            "user.email=identity@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "identity fixture",
        ],
    );
    let owner = owner_fixture_block_on(ProjectOwnerHarness::start(&repo)).unwrap();
    configure_owner_fixture_start_gate(&owner);
    let mut config = super::pane_control_tests::control_launch_config(&repo, 4305);
    config.linked_issue_number = None;
    config.suppress_execution_control = true;
    config.base_branch = None;
    let pause = owner_fixture_block_on(
        owner.pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall),
    )
    .unwrap();
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let id = owner.pane_id("unbound");
    let (launched, (session, before, runtime_path)) = owner_fixture_block_on(async {
        tokio::join!(
            owner.command(OwnerCommand::LaunchAgent {
                id: id.clone(),
                operation_id: "unbound-identity".into(),
                tab_id: id.split_once("::").unwrap().0.into(),
                config: Box::new(config),
                metadata: Box::default(),
                cols: 80,
                rows: 24,
            }),
            async {
                let materialized = pause.wait().await;
                let session = materialized
                    .session
                    .expect("actual owner materialized Session");
                assert!(session.execution_binding.is_none());
                let runtime_path = gwt_agent::runtime_state_path(&sessions, &session.id);
                let mut before =
                    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
                before.source_event = Some("session-start".into());
                let mut predecessor = session.clone();
                predecessor.repo_hash = Some("previous-repo".into());
                predecessor.linked_issue_number = Some(4305);
                predecessor.execution_binding = Some(gwt_agent::SessionExecutionBinding {
                    schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                    session_id: session.id.clone(),
                    repo_hash: "previous-repo".into(),
                    owner_kind: "issue".into(),
                    owner_number: 4305,
                    identity: gwt_agent::ExecutionBindingIdentity {
                        generation_id: "previous-generation".into(),
                        binding_id: "previous-binding".into(),
                        ledger_head_hash: "previous-head".into(),
                    },
                    capability_generation: 1,
                });
                before.execution_identity =
                    gwt_agent::SessionExecutionIdentity::from_session(&predecessor).unwrap();
                before.save(&runtime_path).unwrap();
                pause.release();
                (session, before, runtime_path)
            }
        )
    });
    let launched = launched.unwrap();
    let incarnation = launched["incarnation"].as_u64().unwrap();
    let probe = owner_fixture_block_on(owner.probe(&id, incarnation)).unwrap();
    let observed = gwt_agent::SessionRuntimeState::load(&runtime_path).expect("runtime proof");
    let inventory = gwt::session_inventory::observe_sessions(&repo, &sessions);
    owner_fixture_block_on(owner.command(OwnerCommand::Close { id, incarnation })).unwrap();
    assert!(
        observed.child_pid.is_some(),
        "every agent PTY needs process identity"
    );
    assert!(observed.child_started_at.is_some_and(|started| started > 0));
    assert_eq!(
        observed.host_started_at,
        gwt::process::host_process_start_time(std::process::id())
    );
    assert_eq!(
        observed.source_event, before.source_event,
        "retain hook observations"
    );
    assert_eq!(
        observed.execution_identity, None,
        "observation is not producing authority"
    );
    assert_eq!(inventory.sessions.len(), 1);
    assert_eq!(inventory.sessions[0].session_id, session.id);
    assert!(probe.pty.try_wait().unwrap().is_some());
    let stopped = gwt_agent::SessionRuntimeState::load(&runtime_path).expect("stopped proof");
    assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(stopped.child_pid, observed.child_pid);
    assert_eq!(stopped.child_started_at, observed.child_started_at);
}

#[test]
fn direct_agent_presets_create_and_restart_are_observed_until_stopped() {
    let _env = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo(&repo);
    let fake_codex = write_fake_agent_command(temp.path(), "codex");
    let fake_claude = write_fake_agent_command(temp.path(), "claude");
    for command in [&fake_codex, &fake_claude] {
        write_executable_test_file(
            command,
            if cfg!(windows) {
                "@echo off\r\npause\r\n"
            } else {
                "#!/bin/sh\nread line\n"
            },
        );
    }
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let mut settings = Settings::default();
    pin_launch_agents(
        &mut settings,
        fake_codex.parent().expect("fixture directory"),
    );
    write_profile_config(&temp.path().join("profile-config.toml"), &settings);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "legacy-codex",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    // AC27/40: each direct route needs an enabled member with its fixed tool.
    gwt::agent_names::read_project_roster(&repo, &runtime.sessions_dir)
        .expect("registered Codex member");
    gwt::agent_names::mutate_project_roster(
        &repo,
        &runtime.sessions_dir,
        gwt::agent_names::AgentRosterAction::Add {
            name: None,
            agent_id: Some("claude".into()),
        },
        None,
    )
    .expect("registered Claude member for the second direct route");
    let codex_id = combined_window_id("tab-1", "legacy-codex");
    let (spawn_env, _) = runtime
        .active_profile_spawn_env()
        .expect("launch environment")
        .with_project_root(&repo)
        .into_parts();
    for (command, expected) in [("codex", &fake_codex), ("claude", &fake_claude)] {
        assert_eq!(
            which::which_in(command, spawn_env.get("PATH"), &repo).expect("fixture CLI"),
            *expected,
            "direct presets must launch the long-lived fixture"
        );
    }

    // Resolve the empty cache before launch so a later lazy disk load cannot
    // hide a missing direct-preset Session registration.
    let _ = runtime.launch_wizard_cache.agent_preferences();
    runtime.restart_window_events(&codex_id);
    runtime.create_window_events(&runtime.test_context(), WindowPreset::Claude, None);

    let inventory = gwt::session_inventory::observe_sessions(&repo, &runtime.sessions_dir);
    let uncached_sessions = runtime
        .active_agent_sessions
        .values()
        .filter(|active| {
            runtime
                .launch_wizard_cache
                .session_by_id(&active.session_id)
                .is_none()
        })
        .map(|active| active.session_id.clone())
        .collect::<Vec<_>>();
    let live_windows = runtime.runtimes.keys().cloned().collect::<Vec<_>>();
    for id in &live_windows {
        runtime.stop_window_runtime(id);
    }
    assert!(
        uncached_sessions.is_empty(),
        "direct preset Sessions must be cached before spawn for durable close: {uncached_sessions:?}"
    );
    assert_eq!(
        live_windows.len(),
        2,
        "both direct agent routes must spawn the test commands"
    );
    assert_eq!(
        inventory.sessions.len(),
        2,
        "direct Claude/Codex panes must not be an invisible zero"
    );
    assert!(inventory.uncertainties.is_empty());
    for row in &inventory.sessions {
        assert_eq!(row.launch_origin, gwt_agent::SessionLaunchOrigin::Launch);
        let session = gwt_agent::Session::load(
            &runtime
                .sessions_dir
                .join(format!("{}.toml", row.session_id)),
        )
        .expect("retired Session");
        assert_eq!(session.status, gwt_agent::AgentStatus::Stopped);
        let window = runtime
            .tabs
            .iter()
            .flat_map(|tab| tab.workspace.persisted().windows.iter())
            .find(|window| window.session_id.as_deref() == Some(row.session_id.as_str()))
            .expect("direct agent window");
        assert!(
            window.agent_name.is_some(),
            "direct presets must be named before spawning"
        );
        assert_eq!(
            session.agent_name, window.agent_name,
            "replacing the direct preset Session must retain the window name"
        );
    }
    // `stop_window_runtime` kills without waiting (Issue #3705); Windows
    // TerminateProcess is asynchronous, so the observed children may still be
    // listed briefly. Wait for each exact child to exit before re-observing.
    let deadline = Instant::now() + TEST_PTY_STOP_SETTLEMENT_TIMEOUT;
    for row in &inventory.sessions {
        while gwt::process::exact_pty_process_tree_is_alive(row.child_pid, row.child_started_at) {
            assert!(
                Instant::now() < deadline,
                "stopped direct agent child {} did not exit before the deadline",
                row.child_pid
            );
            thread::sleep(TEST_PTY_STOP_SETTLEMENT_POLL_INTERVAL);
        }
    }
    assert!(
        gwt::session_inventory::observe_sessions(&repo, &runtime.sessions_dir)
            .sessions
            .is_empty()
    );
}

#[test]
fn production_bound_agent_launch_rejects_replaced_identity_without_runtime_sidecar_rewrite() {
    use gwt::project_runtime::events::OwnerCommand;
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _custom = ScopedEnvVar::unset(gwt_agent::DISABLE_GLOBAL_CUSTOM_AGENTS_ENV);
    let (owner, source, _) = managed_bound_resume_source(temp.path());
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let runtime_path = gwt_agent::runtime_state_path(&sessions, &source.id);
    let sentinel_bytes = fs::read(&runtime_path).unwrap();
    let session_path = sessions.join(format!("{}.toml", source.id));
    let mut replacement = source.clone();
    replacement.agent_id = gwt_agent::AgentId::Custom("replacement".into());
    replacement.save(&sessions).unwrap();
    let replacement_bytes = fs::read(&session_path).unwrap();
    let id = owner.pane_id("replaced");
    let response = owner_fixture_block_on(owner.command(OwnerCommand::RestoreSavedSession {
        id: id.clone(),
        operation_id: "bound-replaced-source".into(),
        source_session_id: source.id.clone(),
        expected_source: Box::new(source),
        origin: gwt_agent::SessionLaunchOrigin::UserRestart,
        cols: 80,
        rows: 24,
    }))
    .unwrap();
    assert_eq!(
        response["restore_refusal"]["code"], "source_changed",
        "exact saved Session identity must be refused: {response}"
    );
    assert!(response["restore_refusal"]["message"]
        .as_str()
        .unwrap()
        .contains("changed"));
    let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
    assert!(
        inventory["panes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pane| pane["id"] != id),
        "no runtime or active Session is installed for a replacement"
    );
    assert!(inventory["pending"].as_array().unwrap().is_empty());
    assert_eq!(
        fs::read(&runtime_path).unwrap(),
        sentinel_bytes,
        "same-ID refusal cannot rewrite the prior physical proof"
    );
    assert_eq!(
        fs::read(&session_path).unwrap(),
        replacement_bytes,
        "same-ID refusal cannot rewrite the replacement Session"
    );
    owner_fixture_block_on(Arc::try_unwrap(owner).ok().unwrap().shutdown());
}

#[test]
fn production_bound_agent_spawn_failure_never_publishes_exact_runtime_proof() {
    use gwt::project_runtime::{events::OwnerCommand, test_support::OwnerLaunchStage};
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let (owner, caller) = pane_control_tests::control_owner_agent(&project, "caller", 900);
    let config = pane_control_tests::control_launch_config(&project, 42);
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        owner
            .set_pty_start_gate(project.join("missing-actual-pty-start-gate"), Vec::new())
            .await
            .unwrap();
        let checkpoint = owner
            .pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall)
            .await
            .unwrap();
        let id = owner.pane_id("failed-bound");
        let target = id.clone();
        let client = owner.client();
        let launch = tokio::spawn(async move {
            client
                .command(OwnerCommand::LaunchAgent {
                    id: target,
                    operation_id: "bound-real-spawn-failure".into(),
                    tab_id: "bound-failure".into(),
                    config: Box::new(config),
                    metadata: Box::default(),
                    cols: 80,
                    rows: 24,
                })
                .await
        });
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual bound candidate before physical spawn
        let captured = tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .unwrap();
        let session = captured.session.as_ref().unwrap();
        assert!(
            session.execution_binding.is_some(),
            "the real preparation must have bound its Session"
        );
        assert!(captured.pty.is_none(), "the real start gate has not run");
        checkpoint.release();
        let response = launch.await.unwrap();
        let failure = match response {
            Err(error) => error,
            Ok(receipt) => receipt
                .get("compensated_launch_failure")
                .or_else(|| receipt.get("launch_failure"))
                .expect("actual failed owner response")
                .to_string(),
        };
        assert!(
            failure.contains("missing-actual-pty-start-gate"),
            "{failure}"
        );
        let inventory = owner.command(OwnerCommand::ListPanes).await.unwrap();
        assert!(inventory["panes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pane| pane["id"] != id));
        let path = gwt_agent::runtime_state_path(&gwt_core::paths::gwt_sessions_dir(), &session.id);
        match gwt_agent::SessionRuntimeState::load(&path) {
            Ok(runtime) => {
                assert!(runtime.execution_identity.is_none());
                assert!(runtime.runtime_incarnation.is_none());
            }
            Err(error) => assert_eq!(
                error.kind(),
                std::io::ErrorKind::NotFound,
                "exact compensation may remove the candidate but cannot leave unreadable proof"
            ),
        }
    });
    pane_control_tests::control_close(&owner, &caller);
}

#[test]
fn ordinary_bound_runtime_stop_publishes_terminal_proof_only_after_process_exit() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let (owner, probe) = super::pane_control_tests::control_owner_agent(&repo, "ordinary", 42);
    let session = probe.session.as_ref().unwrap();
    let session_id = session.id.as_str();
    let identity = gwt_agent::SessionExecutionIdentity::from_session(session)
        .unwrap()
        .unwrap();
    let child_pid = probe.pty.process_id().unwrap();
    let child_started_at = gwt::process::host_process_start_time(child_pid).unwrap();
    let incarnation = probe.incarnation;
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let running =
        gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(&sessions, session_id))
            .unwrap();
    assert_eq!(running.child_pid, Some(child_pid));
    assert_eq!(running.child_started_at, Some(child_started_at));
    owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::Close {
            id: probe.id.clone(),
            incarnation,
        }),
    )
    .unwrap();
    wait_for_test_pty_stop_settlement(
        &sessions,
        session_id,
        &identity,
        incarnation,
        child_pid,
        child_started_at,
    );
    assert_eq!(
        gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml")))
            .unwrap()
            .status,
        gwt_agent::AgentStatus::Stopped
    );
    let proof =
        gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(&sessions, session_id))
            .unwrap();
    assert_eq!(proof.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(proof.execution_identity.as_ref(), Some(&identity));
    assert_eq!(proof.runtime_incarnation, Some(incarnation));
    assert_eq!(proof.child_pid, Some(child_pid));
    assert_eq!(proof.child_started_at, Some(child_started_at));
    assert!(
        !gwt_agent::manual_handoff_path(&sessions, session_id).exists(),
        "ordinary close must release the successor-only durable fence"
    );
    probe
        .issuer
        .issue_bound(
            &session.worktree_path,
            session_id,
            identity.execution_binding.clone(),
        )
        .expect("ordinary close must keep same-Session resume capability issuable");
}

#[test]
fn app_runtime_runtime_status_stopped_keeps_active_agent_window_for_diagnostics() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some("Process exited".to_string()),
        true,
    );

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })),
        "PTY stop must still update the terminal status"
    );
    assert!(
        runtime.window_lookup.contains_key(&window_id),
        "PTY stop alone must keep the agent window open so diagnostics remain visible"
    );
    assert!(
        runtime.tabs[0].workspace.window("codex-1").is_some(),
        "workspace must retain the stopped agent window"
    );
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
}

#[test]
fn current_incarnation_confirmed_runtime_exit_auto_closes_active_agent_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let pty = owner_runtime_probe(&runtime, &window_id).pty.clone();
    let incarnation = runtime
        .runtimes
        .get(&window_id)
        .expect("current runtime")
        .incarnation;
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let events = runtime.handle_runtime_status_event(
        window_id.clone(),
        incarnation,
        WindowProcessStatus::Stopped,
        Some("Process exited".to_string()),
        true,
    );

    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })));
    assert!(!runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("codex-1").is_none());
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(!runtime.runtimes.contains_key(&window_id));
    assert_eq!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1,
        "the exact current exit queues one detached close finalizer"
    );
    finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("current runtime close finalizer")();
    assert!(
        pty.try_wait().expect("probe reaped child").is_some(),
        "the detached finalizer reaps the exact exited generation"
    );
}

// Issue #3341: an agent that dies mid-turn is only diagnosable if the exit
// receipt outlives the pane. Every terminal-status branch below tears the
// runtime down (and `mark_agent_session_stopped` drops the active session), so
// the receipt has to be persisted on the way through. A clean `exit 0` is the
// case that previously left no durable evidence anywhere.
#[test]
fn app_runtime_clean_agent_exit_persists_the_pty_exit_receipt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (owner, probe, mut runtime, mut events) =
        managed_exit_fixture(temp.path(), 0, "provider finished");
    let _ = finish_managed_exit(&owner, &probe, &mut runtime, &mut events);
    let session_id = &probe.session.as_ref().unwrap().id;
    let reloaded =
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("reload persisted Session");
    assert_eq!(
        reloaded.last_exit_code,
        Some(0),
        "a clean exit 0 must still be recorded, it is the ambiguous case"
    );
    assert_eq!(reloaded.last_exit_signal, None);
    assert!(
        reloaded.last_exited_at.is_some(),
        "the observation time must be persisted so the death is not dated by the write"
    );
    assert_eq!(
        reloaded.status,
        gwt_agent::AgentStatus::Stopped,
        "the exit receipt must not displace the settled Stopped status"
    );
}

// Issue #3341: `PaneStatus` collapses every failure to `Completed(1)`, so
// without the receipt an Error window cannot report which code the agent
// actually returned.
#[test]
fn app_runtime_agent_error_exit_persists_the_real_exit_code() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (owner, probe, mut runtime, mut events) =
        managed_exit_fixture(temp.path(), 7, "provider finished");
    let _ = finish_managed_exit(&owner, &probe, &mut runtime, &mut events);
    let session_id = &probe.session.as_ref().unwrap().id;
    let reloaded =
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("reload persisted Session");
    assert_eq!(reloaded.last_exit_code, Some(7));
}

// Issue #3341: a reader-thread read error reports `Error` without waiting on
// the child, so there is no exit evidence yet. Recording one anyway would
// invent a death that has not been observed.
#[test]
fn app_runtime_unconfirmed_agent_error_records_no_exit_receipt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    save_sample_agent_session_toml(&runtime, temp.path());
    insert_test_pane_runtime(&mut runtime, &window_id);

    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("read error".to_string()),
        false,
    );

    let reloaded = gwt_agent::Session::load(&runtime.sessions_dir.join("session-1.toml"))
        .expect("reload persisted Session");
    assert_eq!(reloaded.last_exit_code, None);
    assert_eq!(reloaded.last_exited_at, None);
    close_test_owner_runtime(&mut runtime, &window_id);
}

// Issue #3274 (SPEC-1921 exact session restore amendment): when a resumed
// agent process exits because the provider no longer has the conversation,
// the final screen output must survive into the persistent window detail.
// The vt100 state is dropped together with the runtime on Error, so a client
// that reconnects later would otherwise face an empty Error window with no
// clue why exact session restore failed.
#[test]
fn app_runtime_agent_error_exit_promotes_exact_resume_failure_diagnostic() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    owner_runtime_probe(&runtime, &window_id)
        .pane
        .lock()
        .expect("pane")
        .process_bytes(b"No conversation found with session ID: resume-target-1\r\n");

    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("Process exited with status 1".to_string()),
        true,
    );

    let detail = runtime
        .window_details
        .get(&window_id)
        .cloned()
        .unwrap_or_default();
    assert!(
        detail.contains("Exact session restore failed"),
        "exact-resume failure must be promoted to an explicit diagnostic, got: {detail}"
    );
    assert!(
        !detail.contains("resume-target-1"),
        "diagnostic must not persist the provider conversation id, got: {detail}"
    );
    assert!(
        !runtime.runtimes.contains_key(&window_id),
        "errored runtime is still torn down after the tail is captured"
    );
}

// Issue #3274: any agent error exit keeps its last screen output in the
// window detail so the failure reason survives reconnects, while non-agent
// process windows keep the plain exit detail.
#[test]
fn app_runtime_agent_error_exit_keeps_last_output_in_window_detail() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    ));
    persisted.windows.push(sample_window(
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    ));
    persisted.next_z_index = 3;
    let agent_tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: temp.path().join("repo"),
        kind: ProjectKind::Git,
        workspace: WindowRegistry::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![agent_tab], Some("tab-1"));
    let agent_window_id = combined_window_id("tab-1", "agent-1");
    let shell_window_id = combined_window_id("tab-1", "shell-1");
    insert_test_pane_runtime(&mut runtime, &agent_window_id);
    insert_test_pane_runtime(&mut runtime, &shell_window_id);
    owner_runtime_probe(&runtime, &agent_window_id)
        .pane
        .lock()
        .expect("agent pane")
        .process_bytes(b"unexpected fatal: config parse error\r\n");
    owner_runtime_probe(&runtime, &shell_window_id)
        .pane
        .lock()
        .expect("shell pane")
        .process_bytes(b"command not found: frobnicate\r\n");

    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        agent_window_id.clone(),
        WindowProcessStatus::Error,
        Some("Process exited with status 1".to_string()),
        true,
    );
    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        shell_window_id.clone(),
        WindowProcessStatus::Error,
        Some("Process exited with status 1".to_string()),
        true,
    );

    let agent_detail = runtime
        .window_details
        .get(&agent_window_id)
        .cloned()
        .unwrap_or_default();
    assert!(
        agent_detail.contains("Process exited with status 1")
            && agent_detail.contains("unexpected fatal: config parse error"),
        "agent error detail must keep the last screen output, got: {agent_detail}"
    );
    assert_eq!(
        runtime
            .window_details
            .get(&shell_window_id)
            .map(String::as_str),
        Some("Process exited with status 1"),
        "non-agent windows keep the plain exit detail"
    );
}

#[test]
fn app_runtime_runtime_hook_running_recovers_active_agent_after_pty_error() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));

    let error_events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("pty stream interrupted".to_string()),
    );

    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("pty stream interrupted")
    );
    assert!(error_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Error,
            ..
        }
    )));

    let recovered_events = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));

    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    assert!(
        !runtime.window_details.contains_key(&window_id),
        "live hook recovery clears the stale PTY error detail"
    );
    assert!(recovered_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
}

#[test]
fn app_runtime_unconfirmed_status_error_without_live_hook_keeps_active_agent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let error_events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("process failed".to_string()),
    );

    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("process failed")
    );
    assert!(error_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Error,
            ..
        }
    )));
}

#[test]
fn app_runtime_duplicate_pty_error_after_live_hook_keeps_active_agent_for_recovery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));

    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("transient pty error".to_string()),
    );
    assert!(runtime.active_agent_sessions.contains_key(&window_id));

    let duplicate_error_events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("transient pty error".to_string()),
    );

    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert!(duplicate_error_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Error,
            ..
        }
    )));
}

/// SPEC-3431 FR-065: a dead agent frees its Issue Monitor slot even while its
/// pane is kept on screen for diagnosis.
///
/// `WindowProcessStatus::Error` on an agent window comes from `try_wait`
/// (`gwt-terminal/src/pane.rs:175-186`), so the process is gone. Keeping the
/// session record is a **display** concern (#3274: show the user the final
/// screen instead of an empty window); the Issue Monitor's slot accounting is
/// a different question and was wrongly gated on the same flag.
///
/// Observed live: an agent hit its provider usage limit and exited. Its last
/// hook state was `Idle`, so `keep_active_agent_session_for_recovery` was true,
/// `agent_failed` was never published, and the row stayed `launched` with the
/// slot held. With the default `max_active = 1` that stops the whole queue —
/// which is exactly what "the PM registers Issues but nothing ever runs" looks
/// like from the outside.
/// SPEC-3431 FR-068: a hook arrival is what advances the activity clock.
///
/// The existing heartbeat call sits on the PTY-status path and fires only when
/// `handle_runtime_status` receives `Running` — which never happens for a
/// working agent: the launch sets `Running` through `set_window_status`
/// (bypassing this handler) and the PTY watcher thread `continue`s while the
/// process lives, speaking only when it exits. So `last_heartbeat` stayed at
/// the value seeded at launch and "stuck detection" degraded into a fixed
/// timer measured from launch.
///
/// Hook arrivals are the real progress signal — `PreToolUse` / `PostToolUse` /
/// `UserPromptSubmit` each mean one unit of work actually happened.
#[test]
fn agent_hook_arrival_refreshes_the_issue_monitor_activity_clock() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    tab.project_root = repo.clone();
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    runtime
        .pending_launch_feedback_contexts
        .insert(window_id.clone(), issue_monitor_feedback(4975));
    let hook = runtime_hook_state_for_event("Running", "PostToolUse", "session-1");
    // The worker is queued: receipt/window evidence must not claim application yet.
    let captured = capture_tracing_events(|| {
        runtime.handle_daemon_runtime_hook_event(hook.clone());
        assert!(
            runtime.last_agent_activity_for_test(&window_id).is_some(),
            "a hook arrival must refresh the activity clock for its window"
        );
        assert_eq!(
            tasks.lock().expect("queued tasks").len(),
            1,
            "RuntimeHook must defer heartbeat lookup and publication to one blocking worker"
        );
        tasks.lock().unwrap().pop().unwrap()();
        let mut unmatched = hook.clone();
        unmatched.gwt_session_id = Some("unmatched-session".into());
        runtime.handle_daemon_runtime_hook_event(unmatched);
        drop(runtime);
    });
    let delivery: Vec<_> = captured
        .iter()
        .filter(|event| event.target == "gwt.hook.delivery")
        .collect();
    let outcomes: Vec<_> = delivery
        .iter()
        .map(|event| {
            (
                event.fields["stage"].as_str(),
                event.fields["outcome"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            ("window", "matched"),
            ("heartbeat", "publish_unconfirmed"),
            ("window", "unmatched")
        ]
    );
    for event in delivery {
        assert_eq!(event.level, Level::INFO);
        assert_eq!(event.fields["source_event"], "PostToolUse");
        assert_eq!(event.fields["occurred_at"], hook.occurred_at);
        assert_eq!(
            event.fields["gwt_session_id"],
            if event.fields["outcome"] == "unmatched" {
                "unmatched-session"
            } else {
                "session-1"
            }
        );
    }
}

/// Issue #4608: the heartbeat throttle is measured from the last heartbeat it
/// let through, not from the last activity it saw. Measuring from activity
/// meant an agent whose hooks arrived less than a minute apart — any agent
/// working steadily — never published again after launch, so its
/// `last_activity_at` froze while it worked.
#[test]
fn issue_monitor_heartbeat_throttle_counts_from_the_last_publication() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let window_id = "tab-1::agent-1";
    let at = |secs: i64| {
        chrono::DateTime::parse_from_rfc3339("2026-09-22T07:58:46Z")
            .expect("timestamp")
            .with_timezone(&chrono::Utc)
            + chrono::Duration::seconds(secs)
    };

    let due = [0, 30, 59, 61, 90, 121]
        .map(|secs| runtime.take_issue_monitor_heartbeat_slot(window_id, at(secs)));

    assert_eq!(due, [true, false, false, true, false, true]);
    assert_eq!(
        runtime.last_agent_activity_for_test(window_id),
        Some(at(121)),
        "every arrival is still recorded as activity"
    );
}

/// Issue #4608 AC-1: the pane's own terminal output is the liveness signal
/// that does not depend on hooks, so the canvas observation the Monitor judges
/// carries when this pane last wrote anything.
#[test]
fn issue_monitor_window_observation_carries_the_last_pane_output_time() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let observed = |runtime: &AppRuntime| {
        runtime
            .issue_monitor_window_snapshot_for_tab("tab-1", "2026-09-22T12:40:00Z")
            .expect("snapshot")
            .windows
            .into_iter()
            .find(|window| window.window_id == window_id)
            .expect("observation")
            .last_output_at
    };
    assert_eq!(observed(&runtime), None, "no output seen yet is unknown");

    let before = chrono::Utc::now() - chrono::Duration::seconds(1);
    runtime.handle_runtime_output(window_id.clone(), b"Working (3s)".to_vec());

    let last_output_at = observed(&runtime).expect("output time recorded");
    let last_output_at = chrono::DateTime::parse_from_rfc3339(&last_output_at)
        .expect("rfc3339")
        .with_timezone(&chrono::Utc);
    assert!(last_output_at >= before, "{last_output_at} < {before}");
}

#[test]
fn issue_4905_window_observation_carries_session_identity_for_hook_delivery() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("codex-1", Some("session-41".into())));
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let snapshot = runtime
        .issue_monitor_window_snapshot_for_tab("tab-1", "2026-10-15T00:00:01Z")
        .unwrap();
    let observed = serde_json::to_value(&snapshot.windows[0]).unwrap();
    assert_eq!(observed["session_id"], "session-41");
}

/// SPEC-3431 FR-067: an agent that exits cleanly also frees its slot.
///
/// FR-030 closed this leak on the `Error` side, but `exit 0` maps to
/// `WindowProcessStatus::Stopped` (`window_state.rs`, `PaneStatus::Completed(0)`)
/// and took a different path: no `agent_failed`, and the auto-close gate
/// required `window_hook_states == Some(Stopped)` — a value
/// `window_state_for_hook_event` can never return, so the window was never
/// closed and no `WindowClosed` control was ever published. The row stayed
/// `launched` holding the slot forever, and with the default `max_active = 1`
/// that stops the whole queue exactly like the Error-side leak did.
#[test]
fn agent_clean_exit_frees_the_monitor_slot_like_an_error_does() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let runtime_for = |status: WindowProcessStatus| {
        let mut tab = sample_project_tab_with_window(
            "tab-1",
            "agent-1",
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        tab.project_root = repo.clone();
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            sample_active_agent_session("tab-1", &window_id),
        );
        let events = runtime.handle_runtime_status_with_exit_confirmation(
            window_id,
            status,
            Some("Process exited".to_string()),
            true,
        );
        events
            .iter()
            .map(|outbound| outbound.event.event_kind().to_string())
            .collect::<std::collections::BTreeSet<_>>()
    };

    // The Error path is the reference: it already tells the Monitor. Compare
    // only the Monitor-facing events — a clean exit auto-closes the window, so
    // it legitimately stops emitting per-window state for a window that is
    // gone, while a kept-for-diagnosis Error window keeps updating.
    let monitor_events = |status| -> std::collections::BTreeSet<String> {
        runtime_for(status)
            .into_iter()
            .filter(|kind| kind.starts_with("issue_monitor"))
            .collect()
    };
    let reference = monitor_events(WindowProcessStatus::Error);
    assert!(
        !reference.is_empty(),
        "precondition: the Error path notifies the Monitor"
    );
    let missing: Vec<_> = reference
        .difference(&monitor_events(WindowProcessStatus::Stopped))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "a clean exit must release the slot just like a crash; missing: {missing:?}"
    );
}

#[test]
fn agent_error_frees_the_monitor_slot_even_when_the_pane_is_kept_for_diagnosis() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    tab.project_root = repo.clone();
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    // A live hook state is what every working agent leaves behind, so this is
    // the normal case rather than an edge case.
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Idle",
        "Stop",
        "session-1",
    ));

    let kept_for_diagnosis = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("You've hit your usage limit.".to_string()),
        true,
    );
    assert!(
        runtime.recoverable_agent_error_windows.contains(&window_id),
        "precondition: the pane is kept on screen for diagnosis"
    );

    // Control: the identical death with no live hook state left behind. This
    // is the path that already notified the Monitor, so it defines what
    // "notified" looks like without coupling the test to an event variant.
    let mut control = sample_runtime(
        temp.path(),
        vec![{
            let mut tab = sample_project_tab_with_window(
                "tab-1",
                "codex-1",
                WindowPreset::Agent,
                WindowProcessStatus::Running,
            );
            tab.project_root = repo.clone();
            tab
        }],
        Some("tab-1"),
    );
    control.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let notified = control.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("You've hit your usage limit.".to_string()),
        true,
    );

    let kinds = |events: &[OutboundEvent]| {
        events
            .iter()
            .map(|outbound| outbound.event.event_kind().to_string())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let missing: Vec<_> = kinds(&notified)
        .difference(&kinds(&kept_for_diagnosis))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "keeping the pane for diagnosis must not swallow the Monitor notification; missing: {missing:?}"
    );
}

/// Issue #3616 AC-1/AC-3/AC-4: a quota-exhausted exit is a typed hold, not the
/// untyped agent failure that turns the Issue terminal and spends an attempt.
#[test]
fn provider_usage_limit_exit_is_typed_as_a_quota_hold() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let failure = runtime.issue_monitor_failure_for_window(
        &window_id,
        CODEX_USAGE_LIMIT_SCREEN,
        gwt_agent::SessionMode::Normal,
    );

    let gwt::IssueMonitorFailure::ProviderUsageLimit {
        provider,
        resets_at,
        evidence,
        ..
    } = failure.clone().expect("a quota notice is a typed failure")
    else {
        panic!("expected a provider usage limit failure, got {failure:?}");
    };
    assert_eq!(provider, "codex");
    assert!(
        resets_at.is_some(),
        "the notice states when access returns; dropping it forces the PM to guess"
    );
    let evidence = evidence.expect("native message fallback retains its refusal evidence");
    assert_eq!(evidence.source, "failure_notice");
    assert_eq!(evidence.window_id.as_deref(), Some(window_id.as_str()));
    assert_eq!(evidence.screen_region.as_deref(), Some("provider_response"));
    assert_eq!(
        evidence.matched_pattern.as_deref(),
        Some("codex_usage_limit")
    );

    let payload = AppRuntime::issue_monitor_agent_failed_payload_with_failure(
        &window_id,
        CODEX_USAGE_LIMIT_SCREEN,
        Some(3510),
        failure.as_ref(),
    );
    assert_eq!(
        payload
            .pointer("/agent_failed/failure/kind")
            .and_then(serde_json::Value::as_str),
        Some("provider_usage_limit")
    );
}

/// Issue #3616: the block is recognized across providers, and the account is
/// attributed from the pane's own agent rather than from the wording.
///
/// The first report was Codex; the recurrence was Claude, whose text shares no
/// distinctive phrase with it. Reading the provider out of the sentence would
/// have named Claude's outage "codex" (or nothing at all).
#[test]
fn a_claude_usage_limit_exit_is_attributed_to_the_panes_own_agent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "claude-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "claude-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.agent_id = "claude".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);

    let failure = runtime.issue_monitor_failure_for_window(
        &window_id,
        CLAUDE_USAGE_LIMIT_SCREEN,
        gwt_agent::SessionMode::Normal,
    );

    let gwt::IssueMonitorFailure::ProviderUsageLimit {
        provider,
        resets_at,
        ..
    } = failure.clone().expect("a quota notice is a typed failure")
    else {
        panic!("expected a provider usage limit failure, got {failure:?}");
    };
    assert_eq!(
        provider, "claude",
        "the pane's agent is the account that ran out"
    );
    assert!(
        resets_at.is_some(),
        "`resets Aug 20 at 6am` carries a usable instant even without a year"
    );
}

/// Issue #3616 AC-2: the pane must not read as DONE.
///
/// `WindowProcessStatus::Stopped` renders as the `DONE` cue, which says the
/// work finished. The account ran out; the conversation is intact and the pane
/// has to stay resumable rather than look complete.
#[test]
fn provider_usage_limit_keeps_the_pane_out_of_the_done_state() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Idle",
        "Stop",
        "session-1",
    ));

    // Refusal evidence also wins when usage telemetry still reads healthy.
    runtime.set_provider_usage_accounts(vec![codex_usage_account(26.0, false)]);

    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some(CODEX_USAGE_LIMIT_SCREEN.to_string()),
        true,
    );

    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Waiting),
        "a quota block is a wait for the provider, not a completed run"
    );
    assert!(
        runtime.active_agent_sessions.contains_key(&window_id),
        "the session must survive so the same work can resume after the reset"
    );
    assert!(
        runtime
            .window_details
            .get(&window_id)
            .is_some_and(|detail| detail.contains("usage limit")),
        "the pane must say why it is waiting; a clean exit used to discard the screen"
    );
}

#[test]
fn app_runtime_manual_launch_wizard_injects_hermes_launch_choices() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _hermes_home = ScopedEnvVar::set("HERMES_HOME", seed_hermes_home_with_profile(temp.path()));
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "feature/demo", None, None)
        .expect("open launch wizard");

    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .wizard
        .view();
    assert_hermes_choices_injected(&view, "open_launch_wizard_for_branch");
}

#[test]
fn app_runtime_knowledge_launch_wizard_injects_hermes_launch_choices() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _hermes_home = ScopedEnvVar::set("HERMES_HOME", seed_hermes_home_with_profile(temp.path()));
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3863,
            "Hermes launch choices",
            &["enhancement"],
            "body",
            "2026-09-02T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_knowledge_launch_wizard_for_base_branch(
            "tab-1",
            &repo,
            "develop",
            3863,
            LinkedIssueKind::Issue,
        )
        .expect("open issue launch wizard");

    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .wizard
        .view();
    assert_hermes_choices_injected(&view, "open_knowledge_launch_wizard_for_base_branch");
}

#[test]
fn app_runtime_start_work_wizard_injects_hermes_launch_choices() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _hermes_home = ScopedEnvVar::set("HERMES_HOME", seed_hermes_home_with_profile(temp.path()));
    let workspace_home = temp.path().join("workspace");
    let _ = init_managed_workspace_with_develop_worktree(&workspace_home);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        workspace_home.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_start_work_for_project("tab-1", &workspace_home)
        .expect("open start work");

    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .wizard
        .view();
    assert_hermes_choices_injected(&view, "open_start_work_for_project");
}

#[test]
fn app_runtime_issue_monitor_configure_profile_wizard_injects_hermes_launch_choices() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _hermes_home = ScopedEnvVar::set("HERMES_HOME", seed_hermes_home_with_profile(temp.path()));
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureProfile,
    );

    let view = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("launch wizard view");
    assert_hermes_choices_injected(view, "open_issue_monitor_configure_profile_wizard_events");
}

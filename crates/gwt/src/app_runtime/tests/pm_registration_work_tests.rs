use super::*;

/// Issue #3607 AC-1 / AC-2 / AC-4: the second store of a split repository must
/// not start its own PM. Store-scoped uniqueness cannot see the first one, so
/// before this gate both stores auto-started and two PMs supervised one
/// repository.
#[test]
fn pm_ensure_refuses_a_second_pm_for_the_same_repository_across_split_stores() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = split_store_repo(temp.path());

    let live_tab = sample_project_tab_with_window_at(
        "tab-live",
        "agent-1",
        repo.main.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let second_tab = sample_project_tab(
        "tab-second",
        "Linked",
        repo.linked.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![live_tab, second_tab], Some("tab-second"));
    let window_id = "tab-live::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-live", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime.active_agent_sessions.insert(window_id, session);

    // The live PM registered in the *first* store, whose PM worktree is a
    // linked worktree of the same repository.
    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo.main);
    fs::create_dir_all(&pm_worktree).expect("pm worktree");
    run_git(
        &repo.main,
        &[
            "worktree",
            "add",
            "--force",
            "-b",
            "pm/live",
            pm_worktree.to_str().expect("pm worktree path"),
        ],
    );
    gwt::pm_registry::try_register_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(&repo.main),
        pm_registration_fixture("pm-session-live", &pm_worktree),
        |_| false,
    )
    .expect("seed registration");

    let events =
        runtime.ensure_pm_agent_for_tab("tab-second", super::super::pm::PmEnsureTrigger::Automatic);

    assert!(
        runtime
            .tab("tab-second")
            .expect("second tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "the second store must not spawn a PM pane for a repository that already has one"
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .is_empty(),
        "no PM launch may be queued for the second store"
    );
    assert!(
        gwt::pm_registry::load_pm_prefs(&gwt::pm_registry::pm_prefs_path_for_repo_path(
            &repo.linked
        ))
        .expect("second store prefs")
        .registration
        .is_none(),
        "the second store must not acquire its own registration"
    );
    assert!(
        !events.is_empty(),
        "AC-2: the refusal focuses the existing PM instead of failing silently"
    );
}

/// A repository whose only PM is dead must still get one: repository-scoped
/// uniqueness blocks duplicates, never recovery.
#[test]
fn pm_ensure_still_spawns_when_the_other_stores_pm_is_not_live() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = split_store_repo(temp.path());
    fs::write(repo.linked.join("LINKED.md"), "linked-only commit\n")
        .expect("write linked-only commit");
    run_git(&repo.linked, &["add", "LINKED.md"]);
    run_git(&repo.linked, &["commit", "-m", "advance linked checkout"]);
    let local_head = git_stdout(&repo.linked, &["rev-parse", "HEAD"]);
    assert_ne!(
        local_head,
        git_stdout(&repo.main, &["rev-parse", "HEAD"]),
        "the fixture must distinguish the calling linked checkout from the main worktree"
    );

    let second_tab = sample_project_tab(
        "tab-second",
        "Linked",
        repo.linked.clone(),
        ProjectKind::Git,
        &[],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![second_tab], Some("tab-second"));

    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo.main);
    run_git(
        &repo.main,
        &[
            "worktree",
            "add",
            "-b",
            "pm/dead",
            pm_worktree.to_str().expect("pm worktree path"),
        ],
    );
    gwt::pm_registry::try_register_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(&repo.main),
        pm_registration_fixture("pm-session-dead", &pm_worktree),
        |_| false,
    )
    .expect("seed dead registration");

    runtime.ensure_pm_agent_for_tab("tab-second", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1,
        "a dead PM elsewhere must not leave the repository without one"
    );
    let linked_pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo.linked);
    assert_eq!(
        git_stdout(&linked_pm_worktree, &["rev-parse", "HEAD"]),
        local_head,
        "without remote evidence, a fresh PM must remain available from local HEAD"
    );
    assert_eq!(
        git_stdout(&linked_pm_worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        gwt::pm_registry::PM_WORKTREE_BRANCH,
        "the local fallback must still materialize the resident PM branch"
    );
    let freshness = gwt::pm_registry::load_pm_prefs(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(&repo.linked),
    )
    .expect("load linked-store PM prefs")
    .worktree_freshness
    .expect("local fallback freshness");
    assert_eq!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        freshness.target_observation,
        gwt::pm_registry::PmWorktreeTargetObservation::Unavailable
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Fetch)
    );
    assert_eq!(freshness.base_ref, "HEAD");
    assert_eq!(freshness.head_sha.as_deref(), Some(local_head.as_str()));
    assert_eq!(freshness.target_sha, None);
}

/// Issue #3607 AC-3: the stopped store was not even open, yet its PM came back
/// because the current store's `workspace.json` still held a window whose
/// Session pointed at that store's `pm/worktree`.
#[test]
fn restore_refuses_a_window_bound_to_another_stores_pm_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let orphan_pm_worktree = foreign_pm_worktree("b19aac38305901f5");

    let mut persisted = empty_workspace_state();
    let mut agent_window =
        sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Stopped);
    agent_window.agent_id = Some("claude".to_string());
    agent_window.session_id = Some("session-foreign-pm".to_string());
    persisted.windows.push(agent_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-current".to_string(),
        title: "Current".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-current"));

    let mut session =
        gwt_agent::Session::new(&orphan_pm_worktree, "work", gwt_agent::AgentId::ClaudeCode);
    session.id = "session-foreign-pm".to_string();
    session.agent_session_id = Some("native-foreign-pm".to_string());
    session.restore_window_on_startup = true;
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.save(&runtime.sessions_dir).expect("save session");

    let events = runtime.restore_open_project_windows("tab-current");

    assert!(
        events.is_empty(),
        "restoring another store's PM worktree must not spawn anything"
    );
    assert!(
        runtime.pending_auto_resume_sources.is_empty(),
        "no resume may be tracked for a foreign PM session"
    );
}

/// The same gate must leave the store's *own* PM restorable — the stale-PM
/// resume path (FR-003) goes through the same primitive.
#[test]
fn restore_still_resumes_the_stores_own_pm_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let own_pm_worktree = create_detached_pm_worktree_fixture(&repo);

    let mut persisted = empty_workspace_state();
    let mut agent_window =
        sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Stopped);
    agent_window.agent_id = Some("claude".to_string());
    agent_window.session_id = Some("session-own-pm".to_string());
    persisted.windows.push(agent_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-current".to_string(),
        title: "Current".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-current"));

    let mut session =
        gwt_agent::Session::new(&own_pm_worktree, "work", gwt_agent::AgentId::ClaudeCode);
    session.id = "session-own-pm".to_string();
    session.agent_session_id = Some("native-own-pm".to_string());
    session.restore_window_on_startup = true;
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.save(&runtime.sessions_dir).expect("save session");
    // Reopened #4143 AC-5/8: the exemption rests on the registration, not on
    // the path. Sitting in the canonical PM worktree is not yet being the
    // resident PM, so until the store names this Session the landed-branch
    // gate refuses it like any other landed checkout.
    assert_eq!(
        runtime.restore_admission(&session, &repo, None),
        Err(super::super::startup::RestoreRefusal::LandedWorktree),
        "a canonical PM path alone does not authorize a landed restore"
    );
    gwt::pm_registry::try_register_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(&repo),
        pm_registration_fixture("session-own-pm", &own_pm_worktree),
        |_| false,
    )
    .expect("register the store's own PM");

    runtime.restore_open_project_windows("tab-current");

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(
        !events.is_empty(),
        "the store's own PM must still restore through the shared primitive"
    );
    assert!(runtime
        .pending_auto_resume_sources
        .values()
        .any(|source| source == "session-own-pm"));
}

/// Issue #4394 AC-1 / AC-5: a GUI restart brings back exactly one PM window.
///
/// The incident canvas held three windows in this store's own `pm/worktree`
/// while `pm.json` named only one of them. The #3607 gate compares stores, so
/// all three came back and two unregistered PMs kept posting rulings.
#[test]
fn restore_brings_back_only_the_registered_pm_of_the_stores_pm_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let own_pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let registered = "session-registered-pm";
    let orphans = ["session-orphan-pm-a", "session-orphan-pm-b"];

    let mut persisted = empty_workspace_state();
    for (index, session_id) in std::iter::once(registered).chain(orphans).enumerate() {
        let mut window = sample_window(
            &format!("agent-{}", index + 1),
            WindowPreset::Agent,
            WindowProcessStatus::Stopped,
        );
        window.agent_id = Some("claude".to_string());
        window.session_id = Some(session_id.to_string());
        persisted.windows.push(window);
    }
    persisted.next_z_index = 4;
    let tab = ProjectTabRuntime {
        id: "tab-current".to_string(),
        title: "Current".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-current"));

    for session_id in std::iter::once(registered).chain(orphans) {
        let mut session =
            gwt_agent::Session::new(&own_pm_worktree, "work", gwt_agent::AgentId::ClaudeCode);
        session.id = session_id.to_string();
        session.agent_session_id = Some(format!("native-{session_id}"));
        session.restore_window_on_startup = true;
        session.update_status(gwt_agent::AgentStatus::Stopped);
        session.save(&runtime.sessions_dir).expect("save session");
    }
    gwt::pm_registry::try_register_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(&repo),
        pm_registration_fixture(registered, &own_pm_worktree),
        |_| false,
    )
    .expect("register one PM");

    runtime.restore_open_project_windows("tab-current");

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(!events.is_empty(), "the registered PM must still restore");
    assert_eq!(
        runtime
            .pending_auto_resume_sources
            .values()
            .collect::<Vec<_>>(),
        vec![registered],
        "only the registered PM may be resumed"
    );
    for orphan in orphans {
        let session = gwt_agent::Session::load_and_migrate(
            &runtime.sessions_dir.join(format!("{orphan}.toml")),
        )
        .expect("load orphan session");
        assert!(
            !session.restore_window_on_startup,
            "{orphan} must not come back on the next startup either"
        );
        assert!(
            runtime
                .tab("tab-current")
                .expect("tab")
                .workspace
                .persisted()
                .windows
                .iter()
                .all(|window| window.session_id.as_deref() != Some(orphan)),
            "{orphan} must not keep a PM placeholder on the canvas"
        );
    }
}

/// Issue #4394 AC-2: succession retires the replaced PM Session for restore,
/// even when it ended without `pm.stop` (crash, GUI restart).
#[test]
fn pm_registration_succession_marks_the_replaced_session_unrestorable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    fs::create_dir_all(&pm_worktree).expect("create PM worktree");

    let mut previous =
        gwt_agent::Session::new(&pm_worktree, "work", gwt_agent::AgentId::ClaudeCode);
    previous.id = "session-previous-pm".to_string();
    previous.restore_window_on_startup = true;
    previous.save(&runtime.sessions_dir).expect("save session");
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("session-previous-pm", &pm_worktree),
        |_| false,
    )
    .expect("register the previous PM");

    runtime.register_pm_after_launch(&repo, "session-successor-pm", "claude", &pm_worktree);

    assert_eq!(
        gwt::pm_registry::load_pm_prefs(&prefs_path)
            .expect("load prefs")
            .registration
            .map(|registration| registration.session_id),
        Some("session-successor-pm".to_string())
    );
    let previous = gwt_agent::Session::load_and_migrate(
        &runtime.sessions_dir.join("session-previous-pm.toml"),
    )
    .expect("load previous session");
    assert!(
        !previous.restore_window_on_startup,
        "the replaced PM must not be restorable"
    );
    assert_eq!(previous.status, gwt_agent::AgentStatus::Stopped);
}

#[test]
fn pm_ensure_respects_auto_start_opt_out() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    // FR-002 negative: the opt-out must suppress the auto-start entirely.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    disable_pm_auto_start(&repo);

    let events =
        runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    // FR-026: the opt-out still owes the settings panel the current state, so
    // the one permitted event is the pm_status snapshot — never a spawn.
    assert!(
        events
            .iter()
            .all(|outbound| matches!(outbound.event, BackendEvent::PmStatus { .. })),
        "opt-out must emit nothing but the PM status snapshot"
    );
    assert_eq!(events.len(), 1);
    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .is_empty());
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_launches
        .is_empty());
}

#[test]
fn pm_ensure_spawns_fresh_pm_when_unregistered() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_pm_ensure_spawns_from_default_branch("develop", true);
}

#[test]
fn pm_ensure_spawns_from_main_without_valid_cached_origin_head() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_pm_ensure_spawns_from_default_branch("main", false);
}

#[test]
fn pm_ensure_spawns_from_master_with_cached_origin_head() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_pm_ensure_spawns_from_default_branch("master", true);
}

/// Issue #4375 (AC-1): preparing the PM worktree runs `git worktree add` and
/// `git fetch`, whose cost scales with the repository's worktree count. It must
/// not run on the GUI event loop, so the ensure hands the preparation to a
/// blocking worker and the pane appears when the completion event is
/// dispatched, not inside the ensure call itself.
#[test]
fn pm_ensure_prepares_the_worktree_off_the_event_loop() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events =
        runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "the PM pane must not spawn before the worktree preparation reports back"
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .is_empty(),
        "no launch may be tracked before the preparation reports back"
    );
    assert!(
        events
            .iter()
            .all(|outbound| matches!(outbound.event, BackendEvent::PmStatus { .. })),
        "the on-loop part of the ensure only reports PM status"
    );

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(
        !events.is_empty(),
        "the prepared spawn emits workspace events"
    );
    let windows = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .clone();
    assert_eq!(windows.len(), 1, "exactly one PM pane spawned");
    assert_eq!(windows[0].preset, WindowPreset::Agent);
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1,
        "the prepared spawn tracks its launch for registration at completion"
    );
    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    assert!(
        pm_worktree.join(".git").exists(),
        "the worker must have prepared the canonical PM worktree at {}",
        pm_worktree.display()
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_worktree_preparations
            .is_empty(),
        "consuming the completion releases the in-flight gate"
    );
}

/// Issue #4375: moving the preparation off the loop must not cost the PM its
/// singleton property. While one preparation is in flight a second ensure must
/// not start another, which would land two PM panes for one repository.
/// Issue #4375 (AC-2): `refresh_pm_worktree_for_repo_path` must never run on
/// the GUI event loop thread. This is judged by execution path rather than by
/// startup telemetry: with the blocking spawner holding its queue, the ensure
/// returns having created no Git state at all, and the worktree only appears
/// once the queued task is actually run. If the refresh were still inline, the
/// worktree would exist the moment the ensure returned.
#[test]
fn pm_ensure_never_touches_git_on_the_event_loop_thread() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    // The queue never runs on its own, so "did Git run during the ensure?" has
    // a deterministic answer instead of a race with a worker thread.
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    assert!(
        !pm_worktree.exists(),
        "fixture starts without a PM worktree"
    );

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    assert!(
        !pm_worktree.exists(),
        "the ensure must not have run `git worktree add` on the calling thread; \
         the PM worktree at {} already exists",
        pm_worktree.display()
    );
    let task = {
        let mut tasks = queued.lock().expect("queued tasks");
        assert_eq!(
            tasks.len(),
            1,
            "the Git refresh must be handed to the blocking spawner"
        );
        tasks.remove(0)
    };

    task();

    assert!(
        pm_worktree.join(".git").exists(),
        "running the queued task is what materializes the PM worktree at {}",
        pm_worktree.display()
    );
}

#[test]
fn pm_ensure_refuses_a_second_in_flight_worktree_preparation() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    // Hold every preparation in the queue so the gate is observed while the
    // first one is still in flight rather than after it has finished.
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);
    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    assert_eq!(
        queued.lock().expect("queued tasks").len(),
        1,
        "only one PM worktree preparation may be in flight per repository"
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_worktree_preparations
            .contains(&repo),
        "the in-flight gate names the repository being prepared"
    );
}

/// Issue #4375 (AC-3): a preparation that fails off the loop must stay visible.
/// The synchronous path only logged, so a PM that never appeared — a Git error,
/// a full disk — was indistinguishable from one that was simply disabled.
#[test]
fn failed_pm_worktree_preparation_is_reported_and_spawns_nothing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pending_pm_worktree_preparations
        .insert(repo.clone());

    let events = runtime.handle_pm_worktree_prepared(
        super::super::pm::PmWorktreeContinuation::FreshSpawn {
            tab_id: "tab-1".to_string(),
            project_root: repo.clone(),
        },
        Err("git worktree add failed: No space left on device".to_string()),
    );

    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "a failed preparation must not leave a pane behind"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_launches
        .is_empty());
    assert!(
        !runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_worktree_preparations
            .contains(&repo),
        "a failed preparation releases the gate so a later ensure can retry"
    );
    let (level, message) = events
        .iter()
        .find_map(|outbound| match &outbound.event {
            BackendEvent::IssueMonitorToast { level, message, .. } => {
                Some((level.clone(), message.clone()))
            }
            _ => None,
        })
        .expect("the failure must reach the notification center");
    assert_eq!(level, "error");
    assert!(
        message.contains("No space left on device"),
        "the toast must carry the underlying Git failure: {message}"
    );
}

/// Issue #4375 (AC-4): the restore drain reports its own breakdown, so a
/// startup stall is attributable to the phase that caused it instead of only to
/// the dispatch total.
#[test]
fn restore_drain_stall_warning_names_the_phase_that_blocked_the_loop() {
    use super::super::startup::restore_drain_stall_warning;

    assert!(
        restore_drain_stall_warning(&[("resume", 12), ("pm_ensure", 8)]).is_none(),
        "a drain inside the budget produces no warning"
    );
    assert!(
        restore_drain_stall_warning(&[("pm_ensure", 99)]).is_none(),
        "99ms stays inside the 100ms event-loop budget"
    );
    assert!(
        restore_drain_stall_warning(&[("pm_ensure", 100)]).is_some(),
        "100ms is the point the drain is reported as blocking"
    );

    let warning = restore_drain_stall_warning(&[("resume", 12), ("pm_ensure", 3_687)])
        .expect("a phase over the budget must be reported");
    assert!(
        warning.contains("pm_ensure 3687ms"),
        "the warning names the phase and its cost: {warning}"
    );
    assert!(
        !warning.contains("resume 12ms"),
        "phases inside the budget stay out of the warning: {warning}"
    );
}

/// Issue #4520 AC-4: with an isolated HOME and no restored window, the restore
/// drain — including the queued startup PM ensure that once ran `git worktree
/// add` on the loop for 3,687 ms (#4375) — finishes inside 500 ms.
///
/// Flake tolerance: the budget is judged on the fastest of three independent
/// runtimes, so a single scheduling hiccup on a saturated host cannot fail it,
/// while a drain that is structurally slow is slow in every attempt. The
/// load-independent half of the guarantee is the Git spawn count: the drain
/// must start no logged Git process on the loop thread at all.
#[test]
fn restore_drain_with_no_restored_windows_finishes_inside_500ms() {
    const RESTORE_DRAIN_BUDGET: Duration = Duration::from_millis(500);
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let mut fastest = Duration::MAX;
    for _ in 0..3 {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        runtime.bootstrap();
        assert_eq!(runtime.pending_startup_pm_tabs, vec!["tab-1".to_string()]);

        let git_spawns = gwt_core::process::thread_git_spawn_count();
        let started = Instant::now();
        runtime.startup_auto_resume_ready_events(canvas_bounds());
        fastest = fastest.min(started.elapsed());
        assert_eq!(
            gwt_core::process::thread_git_spawn_count() - git_spawns,
            0,
            "the restore drain must not run Git on the GUI event loop"
        );

        // Let the off-loop preparation finish before its tempdir is removed.
        drain_pm_worktree_preparation(&mut runtime, &recorded_events);
    }
    assert!(
        fastest < RESTORE_DRAIN_BUDGET,
        "restore_drain took {fastest:?} in its fastest of 3 runs (budget {RESTORE_DRAIN_BUDGET:?})"
    );
}

#[test]
fn pm_ensure_refreshes_existing_unregistered_pm_worktree_to_latest_origin_develop() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let commit_a = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    let commit_b = advance_origin_develop_by_one_commit(&repo, &origin);
    assert_ne!(
        commit_a, commit_b,
        "the fixture must contain commits A and B"
    );
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(
        gwt::pm_registry::pm_worktree_path_for_repo_path(&repo),
        pm_worktree,
        "fresh spawn must reuse the canonical PM worktree path"
    );
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        gwt::pm_registry::PM_WORKTREE_BRANCH,
        "the refreshed PM worktree must run on its resident branch"
    );
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        commit_b,
        "an unregistered PM must start from the latest origin/develop"
    );
}

#[test]
fn pm_refresh_resolves_managed_asset_collisions_from_old_head() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let seed = temp.path().join("seed");
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let branch_head = git_stdout(&repo, &["rev-parse", "refs/heads/develop"]);
    let paths = [
        ".claude/skills/gwt-agent/SKILL.md",
        ".codex/skills/gwt-agent/SKILL.md",
        ".claude/commands/gwt-agent.md",
    ];
    gwt_skills::update_git_exclude(&pm_worktree).expect("exclude managed paths");
    for relative in paths {
        for root in [&pm_worktree, &seed] {
            fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
        }
        fs::write(pm_worktree.join(relative), "old generated asset\n").unwrap();
        fs::write(seed.join(relative), "new tracked asset\n").unwrap();
        run_git(&seed, &["add", "--force", "--", relative]);
    }
    run_git(
        &seed,
        &["commit", "-qm", "track managed assets in commit B"],
    );
    run_git(&seed, &["push", origin.to_str().unwrap(), "develop"]);
    let commit_b = git_stdout(&seed, &["rev-parse", "HEAD"]);

    let outcome = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo)
        .expect("refresh must return its freshness");

    assert!(
        outcome.is_fresh(),
        "managed-only collisions must not remain stale: {outcome:?}"
    );
    assert_eq!(git_stdout(&pm_worktree, &["rev-parse", "HEAD"]), commit_b);
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "refs/heads/develop"]),
        branch_head
    );
    for relative in paths {
        assert_eq!(
            fs::read_to_string(pm_worktree.join(relative)).unwrap(),
            "new tracked asset\n"
        );
    }
    for relative in [
        ".claude/skills/gwt-pm/SKILL.md",
        ".codex/skills/gwt-pm/SKILL.md",
        ".claude/settings.local.json",
        ".codex/hooks.json",
    ] {
        assert!(
            pm_worktree
                .parent()
                .unwrap()
                .join("runtime")
                .join(relative)
                .is_file(),
            "missing regenerated runtime asset {relative}"
        );
    }

    let commit_c = advance_origin_develop_by_one_commit(&repo, &origin);
    let repeated = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).unwrap();
    assert!(
        repeated.is_fresh(),
        "refresh must accept its regenerated assets: {repeated:?}"
    );
    assert_eq!(git_stdout(&pm_worktree, &["rev-parse", "HEAD"]), commit_c);
}

/// SPEC #4486 AC-1/AC-2: a Claude Code plugin layout tracks `.claude/` entries
/// as links into the same checkout, including directory links and multi-hop
/// chains. Advancing the PM onto such a base and advancing again must stay
/// fresh, keep every link a link, leave the link targets untouched, and still
/// register the PM launch.
#[cfg(unix)]
#[test]
fn pm_refresh_stays_fresh_across_tracked_in_worktree_plugin_symlinks() {
    use std::os::unix::fs::symlink;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let seed = temp.path().join("seed");
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).expect("seed PM worktree");

    let plugin = ".claude-plugin/plugins/tool";
    let files = [
        (format!("{plugin}/agents/helper.md"), "helper agent\n"),
        (format!("{plugin}/commands/status.md"), "status command\n"),
        (
            format!("{plugin}/skills/tool-usage/SKILL.md"),
            "usage skill\n",
        ),
    ];
    for (relative, body) in &files {
        fs::create_dir_all(seed.join(relative).parent().unwrap()).unwrap();
        fs::write(seed.join(relative), body).unwrap();
    }
    let links = [
        (
            ".agents/skills/tool-usage",
            "../../.claude-plugin/plugins/tool/skills/tool-usage",
        ),
        // Multi-hop: .claude -> .agents -> .claude-plugin, all inside the checkout.
        (
            ".claude/skills/tool-usage",
            "../../.agents/skills/tool-usage",
        ),
        // A project link that happens to use the gwt- prefix is still project-owned.
        (
            ".claude/skills/gwt-tool-extra",
            "../../.agents/skills/tool-usage",
        ),
        (
            ".claude/agents/helper.md",
            "../../.claude-plugin/plugins/tool/agents/helper.md",
        ),
        (
            ".claude/commands/status.md",
            "../../.claude-plugin/plugins/tool/commands/status.md",
        ),
    ];
    for (relative, target) in links {
        fs::create_dir_all(seed.join(relative).parent().unwrap()).unwrap();
        symlink(target, seed.join(relative)).unwrap();
    }
    run_git(&seed, &["add", "--all"]);
    run_git(
        &seed,
        &["commit", "-qm", "add plugin layout with in-tree links"],
    );
    run_git(&seed, &["push", origin.to_str().unwrap(), "develop"]);
    let commit_b = git_stdout(&seed, &["rev-parse", "HEAD"]);

    let outcome = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo)
        .expect("refresh must return its freshness");
    assert!(
        outcome.is_fresh(),
        "in-worktree plugin links must not block PM refresh: {outcome:?}"
    );
    assert_eq!(git_stdout(&pm_worktree, &["rev-parse", "HEAD"]), commit_b);

    let commit_c = advance_origin_develop_by_one_commit(&repo, &origin);
    let repeated = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).unwrap();
    assert!(
        repeated.is_fresh(),
        "a second refresh across the plugin links must stay fresh: {repeated:?}"
    );
    assert_eq!(git_stdout(&pm_worktree, &["rev-parse", "HEAD"]), commit_c);

    for (relative, target) in links {
        assert_eq!(
            fs::read_link(pm_worktree.join(relative)).unwrap(),
            PathBuf::from(target),
            "project link {relative} must stay a link"
        );
    }
    for (relative, body) in &files {
        assert_eq!(
            fs::read_to_string(pm_worktree.join(relative)).unwrap(),
            *body
        );
    }
    assert_eq!(
        git_stdout(
            &pm_worktree,
            &[
                "status",
                "--porcelain",
                "--",
                ".claude",
                ".agents",
                ".claude-plugin"
            ]
        ),
        "",
        "PM refresh must not modify project-owned plugin files"
    );
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    assert_eq!(
        gwt::pm_registry::load_pm_prefs(&prefs_path)
            .unwrap()
            .worktree_freshness
            .map(|state| state.state),
        Some(gwt::pm_registry::PmWorktreeFreshnessState::Fresh)
    );

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.register_pm_after_launch(&repo, "pm-plugin-links", "claude", &pm_worktree);
    assert_eq!(
        gwt::pm_registry::load_pm_prefs(&prefs_path)
            .unwrap()
            .registration
            .map(|registration| registration.session_id),
        Some("pm-plugin-links".to_string())
    );
}

// Issue #4564: the fixture used to write a *file* at `runtime/.claude/skills/
// gwt-pm`. That only obstructed on Unix, where snapshotting the generated
// `gwt-pm/SKILL.md` leaf through a file fails with ENOTDIR. Windows reports
// the same probe as NotFound, after which the prune legitimately removes the
// stray non-bundled `gwt-*` entry and regeneration succeeds.
#[test]
fn pm_refresh_restores_old_checkout_and_assets_when_regeneration_fails() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    assert_pm_refresh_failure_restores_old_checkout_and_assets(temp.path(), false);
}

/// Issue #4448: the advance is now a fast-forward merge of the resident
/// branch, so the tree transition is blocked by an untracked file the incoming
/// commit would overwrite rather than by a `post-checkout` hook.
#[test]
fn pm_refresh_restores_old_checkout_when_the_fast_forward_is_blocked() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    assert_pm_refresh_failure_restores_old_checkout_and_assets(temp.path(), true);
}

#[test]
fn pm_refresh_preserves_colliding_legacy_work_history_in_readable_shards() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let seed = temp.path().join("seed");
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let relative = ".gwt/work/events.jsonl";
    let upstream = "{\"id\":\"upstream\",\"work_item_id\":\"work-pm\",\"kind\":\"update\",\"updated_at\":\"2026-08-01T00:00:00Z\"}\n";
    let local = " {\"id\":\"local\",\"work_item_id\":\"work-pm\",\"kind\":\"update\",\"updated_at\":\"2026-08-02T00:00:00Z\",\"future_field\":{\"keep\":true}} \n";
    let future = "{\"id\":\"future\",\"work_item_id\":\"work-pm\",\"kind\":\"future_kind\",\"updated_at\":\"2026-08-03T00:00:00Z\",\"future_field\":42}\n";
    for root in [&pm_worktree, &seed] {
        fs::create_dir_all(root.join(".gwt/work")).unwrap();
    }
    fs::write(
        pm_worktree.join(relative),
        format!("{upstream}{local}{future}"),
    )
    .unwrap();
    fs::write(seed.join(relative), upstream).unwrap();
    run_git(&seed, &["add", "--force", "--", relative]);
    run_git(&seed, &["commit", "-qm", "track upstream Work history"]);
    run_git(&seed, &["push", origin.to_str().unwrap(), "develop"]);
    let target = git_stdout(&seed, &["rev-parse", "HEAD"]);

    let outcome = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).unwrap();

    assert!(
        outcome.is_fresh(),
        "durable Work history must be preserved before refresh: {outcome:?}"
    );
    assert_eq!(git_stdout(&pm_worktree, &["rev-parse", "HEAD"]), target);
    assert_eq!(
        fs::read_to_string(pm_worktree.join(relative)).unwrap(),
        upstream
    );
    let snapshot = tracked_work_event_store_snapshot(&pm_worktree);
    for original in [local, future] {
        assert!(
            snapshot
                .shards
                .iter()
                .any(|(path, bytes)| { path.contains('/') && bytes == original.as_bytes() }),
            "original record must remain in a reader-visible immutable shard: {original}"
        );
    }
    assert!(matches!(
        gwt_core::workspace_projection::decode_workspace_work_event_line(future.as_bytes())
            .unwrap(),
        gwt_core::workspace_projection::DecodedWorkspaceWorkEvent::Opaque
    ));
}

#[test]
fn pm_refresh_classifies_user_collision_and_preserves_old_checkout() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let seed = temp.path().join("seed");
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let original_head = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    for n in 0..20 {
        let relative = format!("user-work-{n:02}.txt");
        fs::write(pm_worktree.join(&relative), "local user bytes\n").unwrap();
        fs::write(seed.join(&relative), "incoming bytes\n").unwrap();
        run_git(&seed, &["add", "--", &relative]);
    }
    run_git(&seed, &["commit", "-qm", "add user collision targets"]);
    run_git(&seed, &["push", origin.to_str().unwrap(), "develop"]);

    let outcome = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).unwrap();

    assert!(!outcome.is_fresh());
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        original_head
    );
    for n in 0..20 {
        assert_eq!(
            fs::read_to_string(pm_worktree.join(format!("user-work-{n:02}.txt"))).unwrap(),
            "local user bytes\n"
        );
    }
    let reason = outcome.freshness.failure_reason.unwrap();
    assert!(
        reason.contains("user-owned changes"),
        "missing ownership classification: {reason}"
    );
    assert!(reason.contains("20"), "missing collision count: {reason}");
    assert!(
        reason.len() < 800,
        "diagnosis must contain bounded examples: {reason}"
    );
}

#[test]
fn repeated_pm_refresh_does_not_treat_pure_generated_hook_config_as_local_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let seed = temp.path().join("seed");
    gwt_skills::generate_codex_hooks(&seed).expect("generate tracked Codex hook fixture");
    let hook_path = seed.join(".codex/hooks.json");
    let current = fs::read_to_string(&hook_path).expect("read generated hook fixture");
    fs::write(
        &hook_path,
        current.replace("target/debug/gwtd", "target/old/gwtd"),
    )
    .expect("make tracked hook fixture stale but still purely generated");
    run_git(&seed, &["add", ".codex/hooks.json"]);
    run_git(
        &seed,
        &["commit", "-qm", "track stale generated hook config"],
    );
    run_git(
        &seed,
        &["push", origin.to_str().expect("origin path"), "develop"],
    );
    run_git(&repo, &["fetch", "origin", "develop"]);
    run_git(&repo, &["reset", "--hard", "origin/develop"]);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let main_hook_before = fs::read(repo.join(".codex/hooks.json"))
        .expect("read main-checkout hook config before PM refresh");

    gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo)
        .expect("first refresh materializes current managed hooks");
    assert!(
        crate::runtime_support::intake_hook_config_is_disposable(&pm_worktree, ".codex/hooks.json"),
        "the refresh-produced hook diff must contain only gwt-managed content"
    );
    let commit_c = advance_origin_develop_by_one_commit(&repo, &origin);

    let outcome = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo)
        .expect("second refresh must accept its own generated hook diff");

    assert!(outcome.is_fresh(), "{outcome:?}");
    assert_eq!(git_stdout(&pm_worktree, &["rev-parse", "HEAD"]), commit_c);
    assert_eq!(
        fs::read(repo.join(".codex/hooks.json")).expect("read main-checkout hook config"),
        main_hook_before,
        "PM safe-boundary refresh must not rewrite the linked main checkout"
    );
}

#[test]
fn pm_safe_boundary_rejects_a_worktree_owned_by_a_different_project_hash() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo_a = temp.path().join("project-a/repo");
    let repo_b = temp.path().join("project-b/repo");
    init_git_clone_with_origin(&repo_a);
    init_git_clone_with_origin(&repo_b);
    let mismatched_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo_a);
    fs::create_dir_all(mismatched_worktree.parent().expect("PM directory"))
        .expect("create mismatched PM directory");
    run_git(
        &repo_b,
        &[
            "worktree",
            "add",
            "--detach",
            mismatched_worktree
                .to_str()
                .expect("mismatched worktree path"),
        ],
    );
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo_a);
    let sentinel = gwt::pm_registry::PmWorktreeFreshness {
        state: gwt::pm_registry::PmWorktreeFreshnessState::Fresh,
        base_ref: "origin/develop".to_string(),
        head_sha: Some("repo-a-sentinel".to_string()),
        target_sha: Some("repo-a-sentinel".to_string()),
        behind: Some(0),
        target_observation: gwt::pm_registry::PmWorktreeTargetObservation::Fresh,
        checked_at: "2026-08-29T00:00:00Z".to_string(),
        failure_stage: None,
        failure_reason: None,
    };
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.worktree_freshness = Some(sentinel.clone());
    })
    .expect("seed repo A prefs sentinel");

    gwt::pm_registry::refresh_pm_worktree_at_safe_boundary(&mismatched_worktree)
        .expect_err("repo B must not mutate repo A project-state through a shaped PM path");

    assert_eq!(
        gwt::pm_registry::load_pm_prefs(&prefs_path)
            .expect("reload repo A prefs")
            .worktree_freshness,
        Some(sentinel),
        "identity rejection must not mutate the mismatched project's prefs"
    );
}

#[test]
fn pm_process_refresh_rejects_a_foreign_worktree_before_any_mutation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo_a = temp.path().join("project-a/repo");
    let repo_b = temp.path().join("project-b/repo");
    init_git_clone_with_origin(&repo_a);
    init_git_clone_with_origin(&repo_b);
    let foreign_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo_a);
    fs::create_dir_all(foreign_worktree.parent().expect("PM directory"))
        .expect("create foreign PM directory");
    run_git(
        &repo_b,
        &[
            "worktree",
            "add",
            "--detach",
            foreign_worktree.to_str().expect("foreign worktree path"),
        ],
    );
    let legacy_notes = foreign_worktree.join("tasks/todo.md");
    fs::create_dir_all(legacy_notes.parent().expect("legacy notes parent"))
        .expect("create legacy notes parent");
    fs::write(&legacy_notes, b"foreign PM notes must stay here\n").expect("write foreign notes");
    let prior_head = git_stdout(&foreign_worktree, &["rev-parse", "HEAD"]);
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(&repo_a);
    let scratch = project_dir.join("project-state/pm-scratch/tasks/todo.md");
    let identity = project_dir.join("project-state/pm-worktree-identity.json");
    let prefs_path = project_dir.join("project-state/pm.json");
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.worktree_freshness = Some(gwt::pm_registry::PmWorktreeFreshness {
            state: gwt::pm_registry::PmWorktreeFreshnessState::Fresh,
            base_ref: "origin/develop".to_string(),
            head_sha: Some("stale-fresh-sentinel".to_string()),
            target_sha: Some("stale-fresh-sentinel".to_string()),
            behind: Some(0),
            target_observation: gwt::pm_registry::PmWorktreeTargetObservation::Fresh,
            checked_at: "2026-08-29T00:00:00Z".to_string(),
            failure_stage: None,
            failure_reason: None,
        });
    })
    .expect("seed repo A prior Fresh state");

    gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo_a)
        .expect_err("process refresh must reject a foreign canonical-looking PM worktree");

    assert_eq!(
        git_stdout(&foreign_worktree, &["rev-parse", "HEAD"]),
        prior_head
    );
    assert_eq!(
        fs::read(&legacy_notes).expect("foreign notes remain"),
        b"foreign PM notes must stay here\n"
    );
    assert!(
        !scratch.exists(),
        "foreign notes must not migrate into repo A state"
    );
    assert!(
        !identity.exists(),
        "rejected identity must not be persisted"
    );
    let freshness = gwt::pm_registry::load_pm_prefs(&prefs_path)
        .expect("reload repo A PM prefs")
        .worktree_freshness
        .expect("ownership inspection failure freshness");
    assert_eq!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Inspect)
    );
    assert_eq!(
        freshness.head_sha, None,
        "foreign worktree must not be read"
    );

    gwt::pm_registry::cleanup_pm_worktree_for_repo_path(&repo_a, |_, _| false)
        .expect_err("cleanup must reject the same foreign PM worktree before migration/removal");
    assert_eq!(
        git_stdout(&foreign_worktree, &["rev-parse", "HEAD"]),
        prior_head
    );
    assert_eq!(
        fs::read(&legacy_notes).expect("foreign notes remain after cleanup rejection"),
        b"foreign PM notes must stay here\n"
    );
    assert!(!scratch.exists(), "cleanup must not migrate foreign notes");
}

#[test]
fn pm_process_refresh_rejects_a_swapped_linked_worktree_admin_before_mutation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let other_worktree = temp.path().join("other-worktree");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "--detach",
            other_worktree.to_str().expect("other worktree path"),
        ],
    );
    fs::write(
        pm_worktree.join(".git"),
        fs::read(other_worktree.join(".git")).expect("read other linked marker"),
    )
    .expect("swap PM linked-worktree marker");
    let legacy_notes = pm_worktree.join("tasks/todo.md");
    fs::create_dir_all(legacy_notes.parent().expect("legacy notes parent"))
        .expect("legacy notes parent");
    fs::write(&legacy_notes, b"notes survive swapped admin rejection\n").expect("legacy notes");
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.worktree_freshness = Some(gwt::pm_registry::PmWorktreeFreshness {
            state: gwt::pm_registry::PmWorktreeFreshnessState::Fresh,
            base_ref: "origin/develop".to_string(),
            head_sha: Some("stale-fresh-sentinel".to_string()),
            target_sha: Some("stale-fresh-sentinel".to_string()),
            behind: Some(0),
            target_observation: gwt::pm_registry::PmWorktreeTargetObservation::Fresh,
            checked_at: "2026-08-29T00:00:00Z".to_string(),
            failure_stage: None,
            failure_reason: None,
        });
    })
    .expect("seed prior Fresh state");
    let scratch = gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo).join("tasks/todo.md");

    gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo)
        .expect_err("swapped linked-worktree admin must fail closed");

    assert_eq!(
        fs::read(&legacy_notes).expect("legacy notes remain"),
        b"notes survive swapped admin rejection\n"
    );
    assert!(
        !scratch.exists(),
        "inspection rejection must precede scratch migration"
    );
    let freshness = gwt::pm_registry::load_pm_prefs(&prefs_path)
        .expect("reload PM prefs")
        .worktree_freshness
        .expect("inspection failure freshness");
    assert_eq!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Inspect)
    );
    assert_eq!(freshness.head_sha, None, "untrusted admin must not be read");
}

#[cfg(unix)]
#[test]
fn pm_process_refresh_persists_inspect_unknown_when_identity_storage_is_unsafe() {
    use std::os::unix::fs::symlink;

    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).expect("seed PM identity");
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(&repo);
    let identity = project_dir.join("project-state/pm-worktree-identity.json");
    let external = temp.path().join("external-identity.json");
    fs::write(&external, b"external identity must remain unchanged\n").expect("external identity");
    fs::remove_file(&identity).expect("remove seeded identity");
    symlink(&external, &identity).expect("symlink unsafe identity");

    gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo)
        .expect_err("process refresh must reject a symlinked identity file");

    assert!(fs::symlink_metadata(&identity)
        .expect("identity metadata")
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read(&external).expect("external identity remains"),
        b"external identity must remain unchanged\n"
    );
    let freshness = gwt::pm_registry::load_pm_prefs(&project_dir.join("project-state/pm.json"))
        .expect("reload PM prefs")
        .worktree_freshness
        .expect("identity storage failure freshness");
    assert_eq!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Inspect)
    );
}

#[test]
fn pm_safe_boundary_persists_inspect_unknown_for_corrupt_local_identity() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let outcome =
        gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).expect("seed PM identity");
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(&repo);
    let identity = project_dir.join("project-state/pm-worktree-identity.json");
    fs::write(&identity, b"{invalid identity json\n").expect("corrupt local identity");

    gwt::pm_registry::refresh_pm_worktree_at_safe_boundary(&outcome.worktree)
        .expect_err("safe boundary must reject corrupt local identity");

    let freshness = gwt::pm_registry::load_pm_prefs(&project_dir.join("project-state/pm.json"))
        .expect("reload PM prefs")
        .worktree_freshness
        .expect("identity inspection failure freshness");
    assert_eq!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Inspect)
    );
}

#[test]
fn pm_safe_boundary_persists_inspect_unknown_when_git_root_becomes_unreadable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let outcome =
        gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).expect("seed PM identity");
    fs::write(outcome.worktree.join(".git"), b"invalid linked marker\n")
        .expect("corrupt linked-worktree marker");

    gwt::pm_registry::refresh_pm_worktree_at_safe_boundary(&outcome.worktree)
        .expect_err("safe boundary must reject an unreadable Git root");

    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    let freshness = gwt::pm_registry::load_pm_prefs(&prefs_path)
        .expect("reload PM prefs")
        .worktree_freshness
        .expect("Git-root inspection failure freshness");
    assert_eq!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Inspect)
    );
    assert_eq!(freshness.head_sha, None);
}

#[test]
fn pm_safe_boundary_persists_inspect_unknown_for_a_swapped_linked_admin() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let outcome =
        gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo).expect("seed PM identity");
    let other_worktree = temp.path().join("other-worktree");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "--detach",
            other_worktree.to_str().expect("other worktree path"),
        ],
    );
    fs::write(
        outcome.worktree.join(".git"),
        fs::read(other_worktree.join(".git")).expect("read other linked marker"),
    )
    .expect("swap PM linked-worktree marker");

    gwt::pm_registry::refresh_pm_worktree_at_safe_boundary(&outcome.worktree)
        .expect_err("safe boundary must reject a swapped linked-worktree admin");

    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    let freshness = gwt::pm_registry::load_pm_prefs(&prefs_path)
        .expect("reload PM prefs")
        .worktree_freshness
        .expect("linked-admin inspection failure freshness");
    assert_eq!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Inspect)
    );
    assert_eq!(freshness.head_sha, None);
}

#[test]
fn bare_layout_safe_boundary_migrates_an_existing_identity_missing_pm() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let opened_project = temp.path().join("managed-project");
    let (bare_repo, _develop_worktree) =
        init_managed_workspace_with_develop_worktree(&opened_project);
    let worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&opened_project);
    fs::create_dir_all(worktree.parent().expect("PM parent")).expect("PM parent");
    gwt_git::WorktreeManager::new(bare_repo)
        .create_detached("develop", &worktree)
        .expect("create pre-upgrade bare-layout PM worktree");
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(&opened_project);
    let identity = project_dir.join("project-state/pm-worktree-identity.json");
    assert!(!identity.exists(), "fixture must model a pre-identity PM");

    let outcome = gwt::pm_registry::refresh_pm_worktree_at_safe_boundary(&worktree)
        .expect("identity-missing bare-layout safe boundary")
        .expect("canonical PM worktree");

    assert!(outcome.is_fresh(), "{outcome:?}");
    assert!(
        identity.is_file(),
        "safe boundary must seed the durable identity"
    );
    assert_eq!(outcome.worktree, worktree);
}

#[test]
fn bare_layout_uses_one_pm_project_identity_for_refresh_status_and_safe_boundary() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let opened_project = temp.path().join("managed-project");
    let (_bare_repo, _develop_worktree) =
        init_managed_workspace_with_develop_worktree(&opened_project);
    let expected_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&opened_project);
    let expected_prefs = gwt::pm_registry::pm_prefs_path_for_repo_path(&opened_project);

    let outcome = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&opened_project)
        .expect("bare-layout process refresh");
    let boundary = gwt::pm_registry::refresh_pm_worktree_at_safe_boundary(&expected_worktree)
        .expect("bare-layout safe boundary")
        .expect("canonical PM worktree");

    assert_eq!(outcome.worktree, expected_worktree);
    assert_eq!(boundary.worktree, expected_worktree);
    assert!(outcome.is_fresh() && boundary.is_fresh());
    assert!(
        gwt::pm_registry::load_pm_prefs(&expected_prefs)
            .expect("bare-layout PM prefs")
            .worktree_freshness
            .is_some(),
        "spawn, status, and safe-boundary must share the opened project's PM store"
    );
}

#[test]
fn bare_layout_remote_unavailable_materializes_bare_head_for_fresh_spawn() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let opened_project = temp.path().join("managed-project");
    let (bare_repo, _develop_worktree) =
        init_managed_workspace_with_develop_worktree(&opened_project);
    let local_head = git_stdout(&bare_repo, &["rev-parse", "HEAD"]);
    fs::rename(
        opened_project.join(".seed"),
        opened_project.join(".offline-seed"),
    )
    .expect("make the bare repository origin unavailable");

    let outcome = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&opened_project)
        .expect("bare-layout local HEAD must keep a fresh PM spawn available");

    assert_eq!(
        git_stdout(&outcome.worktree, &["rev-parse", "HEAD"]),
        local_head
    );
    assert_eq!(
        git_stdout(&outcome.worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        gwt::pm_registry::PM_WORKTREE_BRANCH
    );
    assert_eq!(
        outcome.freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    );
    assert_eq!(
        outcome.freshness.target_observation,
        gwt::pm_registry::PmWorktreeTargetObservation::Unavailable
    );
    assert_eq!(
        outcome.freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Fetch)
    );
    assert_eq!(
        outcome.freshness.head_sha.as_deref(),
        Some(local_head.as_str())
    );
    assert_eq!(outcome.freshness.base_ref, "HEAD");
    assert_eq!(outcome.freshness.target_sha, None);
}

#[test]
fn pm_ensure_migrates_legacy_notes_before_refreshing_existing_unregistered_pm_worktree() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let legacy_notes = b"legacy PM notes survive refresh\n";
    fs::write(pm_worktree.join("pm-notes.md"), legacy_notes).expect("write legacy PM notes");
    let commit_a = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    let commit_b = advance_origin_develop_by_one_commit(&repo, &origin);
    assert_ne!(
        commit_a, commit_b,
        "the fixture must contain commits A and B"
    );
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    let scratch = gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo);
    assert_eq!(
        fs::read(scratch.join("pm-notes.md")).expect("read migrated PM notes"),
        legacy_notes,
        "legacy PM notes must be preserved in project-state scratch before refresh"
    );
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        gwt::pm_registry::PM_WORKTREE_BRANCH,
        "the refreshed PM worktree must run on its resident branch"
    );
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        commit_b,
        "legacy scratch migration must not prevent refresh to origin/develop"
    );
}

#[test]
fn pm_ensure_externalizes_modified_tracked_legacy_notes_then_restores_project_content() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let seed = temp.path().join("seed");
    fs::create_dir_all(seed.join("tasks")).expect("seed tasks");
    fs::write(seed.join("tasks/todo.md"), "tracked project task\n").expect("tracked task");
    run_git(&seed, &["add", "tasks/todo.md"]);
    run_git(&seed, &["commit", "-qm", "add tracked project task"]);
    run_git(
        &seed,
        &["push", origin.to_str().expect("origin path"), "develop"],
    );
    run_git(&repo, &["fetch", "origin", "develop"]);
    run_git(&repo, &["reset", "--hard", "origin/develop"]);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let local_notes = b"PM-local task update\n";
    fs::write(pm_worktree.join("tasks/todo.md"), local_notes).expect("modify tracked notes");
    let commit_b = advance_origin_develop_by_one_commit(&repo, &origin);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(git_stdout(&pm_worktree, &["rev-parse", "HEAD"]), commit_b);
    assert_eq!(
        fs::read(gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo).join("tasks/todo.md"))
            .expect("externalized PM notes"),
        local_notes
    );
    assert_eq!(
        fs::read_to_string(pm_worktree.join("tasks/todo.md")).expect("restored project content"),
        "tracked project task\n"
    );
}

#[test]
fn pm_ensure_preserves_existing_pm_worktree_and_records_cached_target_when_fetch_fails() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let commit_a = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    let commit_b = advance_origin_develop_by_one_commit(&repo, &origin);
    run_git(&repo, &["fetch", "origin", "develop"]);
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "origin/develop"]),
        commit_b,
        "the fixture must cache commit B before fetch is made unavailable"
    );
    fs::rename(&origin, temp.path().join("offline-origin.git"))
        .expect("make origin temporarily unavailable without changing project identity");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        commit_a,
        "a failed fetch must preserve the existing PM worktree at commit A"
    );
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).expect("load PM prefs");
    let freshness = prefs
        .worktree_freshness
        .expect("fetch failure must persist PM worktree freshness");
    assert!(matches!(
        freshness.state,
        gwt::pm_registry::PmWorktreeFreshnessState::Stale
            | gwt::pm_registry::PmWorktreeFreshnessState::Unknown
    ));
    assert_eq!(
        freshness.target_observation,
        gwt::pm_registry::PmWorktreeTargetObservation::Cached
    );
    assert_eq!(freshness.behind, Some(1), "{freshness:?}");
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::Fetch)
    );
}

#[test]
fn pm_ensure_preserves_tracked_local_work_and_records_local_work_stage() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let commit_a = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    fs::write(pm_worktree.join("README.md"), "PM local tracked bytes\n")
        .expect("write tracked local PM bytes");
    let commit_b = advance_origin_develop_by_one_commit(&repo, &origin);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(git_stdout(&pm_worktree, &["rev-parse", "HEAD"]), commit_a);
    assert_eq!(
        fs::read_to_string(pm_worktree.join("README.md")).expect("read preserved local bytes"),
        "PM local tracked bytes\n"
    );
    let freshness =
        gwt::pm_registry::load_pm_prefs(&gwt::pm_registry::pm_prefs_path_for_repo_path(&repo))
            .expect("PM prefs")
            .worktree_freshness
            .expect("local-work freshness");
    assert_eq!(freshness.target_sha.as_deref(), Some(commit_b.as_str()));
    assert_eq!(freshness.behind, Some(1));
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::LocalWork)
    );
    let reason = freshness.failure_reason.expect("local-work diagnosis");
    assert!(reason.contains("user-owned changes"), "{reason}");
    assert!(reason.contains("README.md"), "{reason}");
}

#[test]
fn pm_ensure_materializes_the_resident_branch_instead_of_a_detached_head() {
    // Issue #4448 AC-1: a detached PM worktree lets refresh move HEAD off the
    // PM's own commits. The canonical materialization must check out the
    // resident branch so Git itself refuses to rewind it.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        gwt::pm_registry::PM_WORKTREE_BRANCH,
        "the PM worktree must run on its resident branch, not a detached HEAD"
    );
}

#[test]
fn pm_refresh_keeps_a_pushed_but_unmerged_pm_commit_on_the_resident_branch() {
    // Issue #4448 AC-2/AC-5: the PM pushes each commit to its own remote ref
    // before opening a PR, which made the commit reachable from
    // `--remotes` and therefore invisible to the detached-only-commit guard.
    // The next refresh then repointed HEAD to origin/develop and the commit
    // silently left the worktree.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);
    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    let pm_commit = commit_pm_worktree_change(
        &pm_worktree,
        "AGENTS.md",
        "PM ruling applied\n",
        "docs(agents): PM ruling",
    );
    run_git(
        &pm_worktree,
        &["push", "-q", "origin", "HEAD:refs/heads/pm/ruling"],
    );
    let target = advance_origin_develop_by_one_commit(&repo, &origin);
    assert_ne!(pm_commit, target);

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);
    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        pm_commit,
        "refresh must not move HEAD off a PM commit origin/develop does not contain"
    );
    assert_eq!(
        fs::read_to_string(pm_worktree.join("AGENTS.md")).expect("read PM commit bytes"),
        "PM ruling applied\n"
    );
    let freshness =
        gwt::pm_registry::load_pm_prefs(&gwt::pm_registry::pm_prefs_path_for_repo_path(&repo))
            .expect("PM prefs")
            .worktree_freshness
            .expect("retained-commit freshness");
    assert_eq!(
        freshness.failure_stage,
        Some(gwt::pm_registry::PmWorktreeRefreshFailureStage::LocalWork),
        "the retained commit must be reported, not silently dropped"
    );
    assert_eq!(freshness.target_sha.as_deref(), Some(target.as_str()));
}

#[test]
fn pm_refresh_fast_forwards_the_resident_branch_when_it_has_no_local_commits() {
    // Issue #4448 AC-2: keeping PM commits must not stop an ordinary refresh.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);
    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    let target = advance_origin_develop_by_one_commit(&repo, &origin);

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);
    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        target,
        "a PM worktree without local commits must still fast-forward"
    );
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        gwt::pm_registry::PM_WORKTREE_BRANCH
    );
    let freshness =
        gwt::pm_registry::load_pm_prefs(&gwt::pm_registry::pm_prefs_path_for_repo_path(&repo))
            .expect("PM prefs")
            .worktree_freshness
            .expect("fresh freshness");
    assert_eq!(freshness.failure_stage, None, "{freshness:?}");
    assert_eq!(freshness.behind, Some(0));
}

#[test]
fn pm_refresh_adopts_a_legacy_detached_worktree_onto_the_resident_branch() {
    // Issue #4448 AC-1/AC-3: PM worktrees already materialized detached must
    // migrate onto the resident branch without losing the commit they hold.
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    run_git(&pm_worktree, &["config", "user.name", "Codex"]);
    run_git(&pm_worktree, &["config", "user.email", "codex@example.com"]);
    let pm_commit = commit_pm_worktree_change(
        &pm_worktree,
        "AGENTS.md",
        "legacy detached PM commit\n",
        "docs(agents): legacy detached PM commit",
    );
    run_git(
        &pm_worktree,
        &["push", "-q", "origin", "HEAD:refs/heads/pm/legacy"],
    );
    advance_origin_develop_by_one_commit(&repo, &origin);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);
    drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        gwt::pm_registry::PM_WORKTREE_BRANCH,
        "the legacy detached worktree must be adopted onto the resident branch"
    );
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        pm_commit,
        "adoption must preserve the commit the detached HEAD held"
    );
}

#[test]
fn pm_ensure_focuses_live_registered_pm_without_refreshing_its_worktree() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let commit_a = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    let commit_b = advance_origin_develop_by_one_commit(&repo, &origin);
    assert_ne!(
        commit_a, commit_b,
        "the remote must advance beyond the live PM"
    );
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    session.worktree_path = pm_worktree.clone();
    runtime.active_agent_sessions.insert(window_id, session);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &pm_worktree),
        |_| false,
    )
    .expect("seed live PM registration");

    runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        commit_a,
        "focusing a live registered PM must not refresh its running worktree"
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .is_empty(),
        "focusing a live PM must not schedule another launch"
    );
}

#[test]
fn persisted_pm_resume_config_reinjects_project_state_scratch_dir() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let mut session = gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::Codex);
    session.agent_session_id = Some("pm-conversation-1".to_string());

    let config = super::super::launch_config_from_persisted_session(&session);

    assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    assert_eq!(
        config.env_vars.get("GWT_PM_SCRATCH_DIR").map(PathBuf::from),
        Some(gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo)),
        "resuming a canonical PM session must restore its project-state scratch path"
    );
}

/// Issue #3965 AC-1 / AC-2 / AC-3: the restore counterpart of
/// `pm_launch_config_resolves_the_configured_agent_and_defaults_on_a_fresh_project`.
/// A restored PM must start under the same PM contract as a fresh spawn, and a
/// persisted `launch_args` that already records the prompt must not double it.
#[test]
fn persisted_pm_resume_config_reinjects_the_pm_bootstrap_prompt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);

    // The reported shape: a Session persisted by an earlier restore, whose
    // `launch_args` lost the bootstrap prompt entirely.
    let mut stripped = gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::ClaudeCode);
    stripped.agent_session_id = Some("pm-conversation-1".to_string());
    stripped.skip_permissions = true;
    stripped.launch_args = vec!["--dangerously-skip-permissions".to_string()];

    let restored = super::super::launch_config_from_persisted_session(&stripped);

    assert_eq!(
        restored.args.iter().filter(|arg| *arg == "$gwt-pm").count(),
        1,
        "a restored PM session must carry the same bootstrap prompt as a fresh spawn: {:?}",
        restored.args
    );

    // A Session persisted by a fresh spawn already records the prompt; the
    // restore must honor it without duplicating it.
    let mut recorded = gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::ClaudeCode);
    recorded.agent_session_id = Some("pm-conversation-2".to_string());
    recorded.skip_permissions = true;
    recorded.launch_args = vec![
        "--dangerously-skip-permissions".to_string(),
        "$gwt-pm".to_string(),
    ];

    let rebuilt = super::super::launch_config_from_persisted_session(&recorded);

    assert_eq!(
        rebuilt.args.iter().filter(|arg| *arg == "$gwt-pm").count(),
        1,
        "restoring a PM session that already recorded the prompt must not duplicate it: {:?}",
        rebuilt.args
    );
}

/// Issue #3965 AC-4: the bootstrap prompt is a PM-role property. Restoring any
/// other Session must keep producing exactly the args it produced before.
#[test]
fn persisted_non_pm_resume_config_gains_no_bootstrap_prompt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("work/issue-1");
    fs::create_dir_all(&worktree).expect("create work worktree");
    assert!(
        !gwt::pm_registry::is_pm_worktree(&worktree),
        "fixture must not be a PM worktree"
    );

    let mut session =
        gwt_agent::Session::new(&worktree, "work/issue-1", gwt_agent::AgentId::ClaudeCode);
    session.agent_session_id = Some("work-conversation-1".to_string());
    session.skip_permissions = true;
    session.launch_args = vec![
        "--dangerously-skip-permissions".to_string(),
        "$gwt-execute #1".to_string(),
    ];

    let config = super::super::launch_config_from_persisted_session(&session);

    assert!(
        !config.args.iter().any(|arg| arg == "$gwt-pm"),
        "a non-PM session must not gain the PM bootstrap prompt: {:?}",
        config.args
    );
    assert!(
        !config.args.iter().any(|arg| arg == "$gwt-execute #1"),
        "restoring a non-PM session must keep its established args: {:?}",
        config.args
    );
}

/// SPEC-3966 AC-3: the restore branch `ensure_pm_agent_events` takes
/// (`spawn_restored_agent_session` -> `launch_config_from_persisted_session`)
/// must hand the agent CLI `--resume <handle>` whenever the PM Session carries
/// one, or the resident PM starts every restart as a blank conversation.
#[test]
fn restored_pm_session_with_a_resume_handle_launches_with_resume() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let mut session = gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::ClaudeCode);
    session.agent_session_id = Some("pm-conversation-3966".to_string());

    let config = super::super::launch_config_from_persisted_session(&session);

    assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    assert_eq!(
        config.resume_session_id.as_deref(),
        Some("pm-conversation-3966")
    );
    assert_eq!(
        config.predecessor_session_id.as_deref(),
        Some(session.id.as_str())
    );
    assert!(
        config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--resume" && pair[1] == "pm-conversation-3966"),
        "restored PM launch must carry --resume <handle>: {:?}",
        config.args
    );
    assert_eq!(
        super::super::launch::resume_handle_unavailable_reason(&session),
        None
    );
}

/// SPEC-3966 AC-4: falling back to a new conversation must state why. The
/// silent fallback is exactly what hid the Windows managed-hook failure for
/// weeks.
#[test]
fn restored_session_without_a_resume_handle_logs_the_reason() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let session = gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::ClaudeCode);
    assert!(session.agent_session_id.is_none());

    let mut config = None;
    let events = capture_tracing_events(|| {
        config = Some(super::super::launch_config_from_persisted_session(&session));
    });

    let config = config.expect("launch config");
    assert_eq!(config.session_mode, gwt_agent::SessionMode::Normal);
    assert!(!config.args.iter().any(|arg| arg == "--resume"));
    let reason = super::super::launch::resume_handle_unavailable_reason(&session)
        .expect("a stated fallback reason");
    assert!(
        reason.contains("no agent_session_id was ever persisted"),
        "unexpected reason: {reason}"
    );
    assert!(
        events.iter().any(|event| {
            event.level == Level::WARN
                && event
                    .fields
                    .get("message")
                    .is_some_and(|message| message.contains("no resume handle"))
                && event
                    .fields
                    .get("reason")
                    .is_some_and(|logged| logged.contains("no agent_session_id was ever persisted"))
        }),
        "the resume fallback must be observable: {events:?}"
    );
}

/// SPEC-3966 AC-5: each restart writes a successor Session record. If the
/// successor is written without the handle, every restart leaves one more
/// unresumable Session behind and the next restore has nothing to resume.
#[test]
fn repeated_restores_never_accumulate_sessions_without_a_resume_handle() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let mut restored = gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::ClaudeCode);
    restored.agent_session_id = Some("pm-conversation-3966".to_string());

    for restart in 0..3 {
        let config = super::super::launch_config_from_persisted_session(&restored);
        assert_eq!(
            config.session_mode,
            gwt_agent::SessionMode::Resume,
            "restart {restart} lost the resume handle"
        );
        let mut successor =
            gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::ClaudeCode);
        successor.session_mode = config.session_mode;
        super::super::launch::apply_resume_identity_to_session(&mut successor, &config);
        assert_eq!(
            successor.exact_resume_session_id(),
            Some("pm-conversation-3966"),
            "restart {restart} wrote a successor Session with no resume handle"
        );
        restored = successor;
    }
}

#[test]
fn restored_autonomous_session_uses_manual_route_only_for_user_requested_restart() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let runner_bin = write_fixture_runners(temp.path(), &["codex", "npx", "bunx"]);

    for (origin, expected_route) in [
        (
            super::super::startup::RestoreOrigin::Automatic,
            gwt_agent::LaunchRoute::Autonomous,
        ),
        (
            super::super::startup::RestoreOrigin::UserRequested,
            gwt_agent::LaunchRoute::Manual,
        ),
    ] {
        let case_root = temp.path().join(format!("{origin:?}"));
        let repo = case_root.join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(&case_root, vec![tab], Some("tab-1"));
        let mut settings = Settings::default();
        pin_launch_agents(&mut settings, &runner_bin);
        settings
            .profiles
            .set_env_var(
                "default",
                "CODEX_HOME",
                case_root.join("codex-home").to_str().expect("Codex home"),
            )
            .expect("isolate Codex state and shared spawn pacing");
        write_profile_config(runtime.profile_config_path.as_deref().unwrap(), &settings);
        runtime.agent_capability_issuer =
            Some(crate::embedded_server::AgentCapabilityIssuer::for_test(
                "http://127.0.0.1:43123/internal/hook-live",
                "ws://127.0.0.1:43124/ws",
                "ws://127.0.0.1:43123/internal/pane-ws",
            ));
        let mut source = gwt_agent::Session::new(&repo, "main", gwt_agent::AgentId::Codex);
        source.agent_session_id = Some("conversation-4217-restart".to_string());
        source.launch_route = gwt_agent::LaunchRoute::Autonomous;
        source.save(&runtime.sessions_dir).expect("save source");
        let source_session_id = source.id.clone();

        runtime.spawn_restored_agent_session("tab-1", source, None, canvas_bounds(), origin);
        wait_for_recorded_event("restore launch preparation", &recorded_events, |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchComplete { .. }
                )
            })
        });
        let recorded = recorded_events.lock().expect("event log");
        let (window_id, result) = recorded
            .iter()
            .find_map(|event| match recorded_project_payload(event) {
                UserEvent::LaunchComplete { window_id, result } => {
                    Some((window_id.clone(), result.as_ref().clone()))
                }
                _ => None,
            })
            .expect("launch completion");
        let completion = result.as_ref().expect("successful restore preparation");
        // Preparation preserves launch authority; completion records the
        // distinct window origin before publishing the running Session.
        let successor =
            gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{}.toml", completion.1)))
                .expect("load restored Session");
        assert_eq!(successor.launch_route, expected_route, "{origin:?}");
        drop(recorded);
        runtime.handle_launch_complete_and_drain(window_id, result);
        let successor =
            gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{}.toml", successor.id)))
                .expect("load completed restore provenance");
        let value = serde_json::to_value(successor).expect("serialize successor");
        assert_eq!(value["restore_source_session_id"], source_session_id);
        assert_eq!(
            value["launch_origin"],
            match origin {
                super::super::startup::RestoreOrigin::Automatic => "automatic_restore",
                super::super::startup::RestoreOrigin::UserRequested => "user_restart",
            }
        );
    }
}

#[test]
fn generic_pm_session_resume_refreshes_before_spawning_the_process() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let commit_a = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    let commit_b = advance_origin_develop_by_one_commit(&repo, &origin);
    assert_ne!(commit_a, commit_b);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let mut session = gwt_agent::Session::new(&pm_worktree, "", gwt_agent::AgentId::Codex);
    session.agent_session_id = Some("pm-conversation-resume".to_string());
    // Issue #4394 AC-1: only the registered PM's Session may resume from the
    // PM worktree.
    gwt::pm_registry::try_register_pm(
        &gwt::pm_registry::pm_prefs_path_for_repo_path(&repo),
        pm_registration_fixture(&session.id, &pm_worktree),
        |_| false,
    )
    .expect("register the resuming PM");

    runtime.spawn_restored_agent_session(
        "tab-1",
        session,
        None,
        canvas_bounds(),
        super::super::startup::RestoreOrigin::Automatic,
    );

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(!events.is_empty(), "PM resume must still spawn its pane");
    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        commit_b,
        "generic startup/restart resume must refresh before process spawn"
    );
}

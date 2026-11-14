use super::*;

#[test]
fn active_work_projection_build_reads_the_work_items_cache_without_deep_copying() {
    // Issue #4234 AC-1 / AC-2: the Workspace rail build used to ask the
    // WorkItems cache for two *owned* deep copies of the parsed works.json per
    // refresh — once to test emptiness and once to render the rows. At
    // repository scale that is the largest per-refresh allocation left in the
    // GUI, and it is paid on every rail rebuild for the life of the process.
    // The cache already hands out an `Arc`, so the build must borrow it.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let (runtime, _events, _window_id) = active_work_off_loop_setup(temp.path(), &repo);
    let work_items_cache = Arc::clone(
        &runtime
            .project_state_for_tab("tab-1")
            .unwrap()
            .work_items_cache,
    );

    let job = runtime
        .active_work_projection_refresh_job(&repo)
        .expect("refresh job");
    let before = work_items_cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .deep_copy_count;
    let refreshed = super::super::run_active_work_projection_refresh(job);
    let after = work_items_cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .deep_copy_count;

    let view = refreshed.view.expect("the rail still builds");
    assert!(
        view.active_works
            .iter()
            .any(|work| work.branch.as_deref() == Some("work/off-loop")),
        "the borrowed projection must still render the recorded Work"
    );
    assert_eq!(
        after - before,
        0,
        "the rail build copied the parsed works.json {} time(s) instead of \
         borrowing the cached Arc",
        after - before
    );
}

#[test]
fn active_work_projection_refresh_off_the_loop_matches_the_on_loop_build() {
    // Issue #4406 AC-6: the off-loop build is the same build. Applying its
    // result on the event loop must install the cache and broadcast without
    // rebuilding anything.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let (mut runtime, _events, _window_id) = active_work_off_loop_setup(temp.path(), &repo);

    // Issue #3777 AC-5: `active_work_projection_for_tab` no longer builds on the
    // event loop at all, so the comparison baseline is the test-only synchronous
    // builder that runs the very same preparation.
    let expected = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("on-loop projection");

    let job = runtime
        .active_work_projection_refresh_job(&repo)
        .expect("refresh job");
    let refreshed = super::super::run_active_work_projection_refresh(job);

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let applied = runtime.apply_active_work_projection_refresh(refreshed);
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "applying an off-loop refresh must not rebuild on the event loop"
    );
    let projection = applied
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjection { projection } => Some(projection.clone()),
            _ => None,
        })
        .expect("broadcast");
    assert_eq!(
        projection
            .active_works
            .iter()
            .map(|work| work.id.clone())
            .collect::<Vec<_>>(),
        expected
            .active_works
            .iter()
            .map(|work| work.id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(projection.active_agents, expected.active_agents);
}

#[test]
fn runtime_hook_terminal_state_refreshes_active_work_off_the_gui_event_loop() {
    // Issue #4406 AC-4: a `RuntimeHook` arrival that ends a pane rebuilt the
    // whole disk-backed rail on the event loop, holding it for up to 35,982ms.
    // The acknowledgement is served from the cache and the rebuild is requested.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let (mut runtime, events, _window_id) = active_work_off_loop_setup(temp.path(), &repo);
    // Seed the cache the cache-only acknowledgement reads from.
    let _ = runtime.active_work_projection_for_tab("tab-1", &runtime.tabs[0]);

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let hook_events = runtime.handle_runtime_hook_event(runtime_hook_state("Error", "session-1"));

    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "a runtime hook must not enter the disk-backed projection builder"
    );
    assert!(
        hook_events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjectionPatch { .. })),
        "the hook still acknowledges the rail from cache: {hook_events:?}"
    );
    assert_eq!(active_work_refresh_requests(&events, &repo), 1);
}

/// SPEC-2014 FR-PERF-003: ProjectTabRuntime caches `main_worktree_root`
/// resolution per tab so the Launch Wizard / Start Work paths do not
/// re-spawn `git rev-parse --git-common-dir` on every open.
#[test]
fn project_tab_runtime_main_worktree_root_caches_resolution() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    gwt_core::process::hidden_command("git")
        .args(["init", repo.to_str().unwrap()])
        .output()
        .expect("git init");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    assert!(
        tab.main_worktree_root_cache.get().is_none(),
        "cache must start empty"
    );

    let first = tab.main_worktree_root();
    let cached = tab
        .main_worktree_root_cache
        .get()
        .expect("cache populated after first access")
        .clone();
    assert_eq!(first, cached);

    let second = tab.main_worktree_root();
    assert_eq!(
        first, second,
        "second call must return the cached resolution"
    );
}

#[test]
fn migration_detected_broadcasts_only_for_pending_tabs() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    fs::create_dir_all(&repo_a).expect("repo-a");
    fs::create_dir_all(&repo_b).expect("repo-b");

    let pending = migration_pending_tab("tab-1", repo_a);
    let mut clean = sample_project_tab("tab-2", "Other", repo_b, ProjectKind::Git, &[]);
    clean.migration_pending = false;
    let runtime = sample_runtime(temp.path(), vec![pending, clean], Some("tab-1"));

    let events = runtime.migration_detected_broadcasts();

    assert_eq!(events.len(), 1, "only pending tabs should broadcast");
    assert!(matches!(
        &events[0],
        OutboundEvent {
            target: DispatchTarget::Project(key),
            event: BackendEvent::MigrationDetected { tab_id, .. },
            ..
        } if tab_id == "tab-1" && Some(key) == runtime.project_key_for_tab(tab_id)
    ));
}

#[test]
fn migration_completion_ignores_reopened_tab_generation() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let new_worktree = project.join("develop");
    fs::create_dir_all(&new_worktree).expect("new worktree");
    let tab = migration_pending_tab("tab-1", project.clone());
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let context = runtime.project_context("tab-1").expect("project context");

    // Reopening the same project preserves its tab ID and key, but not its generation.
    runtime.close_project_tab_events("tab-1");
    runtime
        .tabs
        .push(migration_pending_tab("tab-1", project.clone()));
    runtime.refresh_project_tab_incarnation("tab-1");
    runtime.active_tab_id = Some("tab-1".to_string());

    assert!(runtime
        .handle_migration_progress(&context, gwt_core::migration::MigrationPhase::Bareify, 50,)
        .is_empty());
    let events = runtime.handle_migration_error(
        &context,
        gwt_core::migration::MigrationPhase::Bareify,
        "old failure".to_string(),
        gwt_core::migration::RecoveryState::RolledBack,
    );
    assert!(
        events.is_empty(),
        "stale errors must not reach reopened project"
    );
    assert!(runtime.tabs[0].migration_pending);
    let events = runtime.handle_migration_done(&context, &new_worktree);
    assert!(
        events.is_empty(),
        "stale completion must not reach reopened project"
    );
    assert_eq!(runtime.tabs[0].project_root, project);
    assert!(runtime.tabs[0].migration_pending);
}

#[test]
fn handle_migration_done_repoints_tab_and_emits_project_event() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let new_worktree = project.join("develop");
    fs::create_dir_all(&new_worktree).expect("new worktree");

    let tab = migration_pending_tab("tab-1", project);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let context = runtime.project_context("tab-1").expect("project context");
    let progress = runtime.handle_migration_progress(
        &context,
        gwt_core::migration::MigrationPhase::Bareify,
        50,
    );
    assert!(matches!(
        &progress[..],
        [OutboundEvent {
            target: DispatchTarget::Project(project_key),
            event: BackendEvent::MigrationProgress { percent: 50, .. },
            ..
        }] if project_key == &context.project_key
    ));
    let events = runtime.handle_migration_done(&context, &new_worktree);

    let updated = runtime
        .tabs
        .iter()
        .find(|t| t.id == "tab-1")
        .expect("tab still present");
    let canonical_new = dunce::canonicalize(&new_worktree).unwrap_or_else(|_| new_worktree.clone());
    assert_eq!(updated.project_root, canonical_new);
    assert!(!updated.migration_pending, "pending flag must clear");

    assert!(matches!(
        &events[0],
        OutboundEvent {
            target: DispatchTarget::Project(project_key),
            event: BackendEvent::MigrationDone { tab_id, .. },
            ..
        } if project_key == &context.project_key && tab_id == "tab-1"
    ));
}

#[test]
fn handle_migration_error_clears_pending_and_emits_project_recovery_label() {
    use gwt_core::migration::{MigrationPhase, RecoveryState};
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");

    let tab = migration_pending_tab("tab-1", project);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let context = runtime.project_context("tab-1").expect("project context");
    let events = runtime.handle_migration_error(
        &context,
        MigrationPhase::Bareify,
        "boom".to_string(),
        RecoveryState::RolledBack,
    );

    assert!(
        !runtime
            .tabs
            .iter()
            .find(|t| t.id == "tab-1")
            .unwrap()
            .migration_pending
    );
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Project(project_key),
            event: BackendEvent::MigrationError { tab_id, recovery, phase, .. },
            ..
        } if project_key == &context.project_key && tab_id == "tab-1" && recovery == "rolled_back" && phase == "bareify"
    )));
}

#[test]
fn github_repository_search_parser_maps_gh_json_fields() {
    let raw = r#"[
          {
            "fullName": "akiojin/gwt",
            "description": "Git Worktree Manager",
            "url": "https://github.com/akiojin/gwt",
            "defaultBranch": "develop",
            "visibility": "public",
            "updatedAt": "2026-05-13T00:00:00Z"
          }
        ]"#;

    let repositories =
        super::super::parse_github_repository_search_results(raw).expect("parse gh search json");

    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0].full_name, "akiojin/gwt");
    assert_eq!(
        repositories[0].description.as_deref(),
        Some("Git Worktree Manager")
    );
    assert_eq!(repositories[0].url, "https://github.com/akiojin/gwt");
    assert_eq!(repositories[0].default_branch.as_deref(), Some("develop"));
    assert_eq!(repositories[0].visibility.as_deref(), Some("public"));
    assert_eq!(
        repositories[0].updated_at.as_deref(),
        Some("2026-05-13T00:00:00Z")
    );
}

#[test]
fn clone_project_done_opens_workspace_home_and_broadcasts_done() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let workspace_home = temp.path().join("sample");
    let bare_repo = workspace_home.join("sample.git");
    fs::create_dir_all(&workspace_home).expect("workspace home");
    let output = gwt_core::process::hidden_command("git")
        .args(["init", "--bare", bare_repo.to_str().expect("bare path")])
        .output()
        .expect("git init --bare");
    assert!(
        output.status.success(),
        "git init --bare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (mut runtime, recorded_events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    assert!(
        runtime
            .handle_clone_project_done(&workspace_home)
            .is_empty(),
        "the clone completion is prepared off-thread"
    );
    let events = commit_pending_project_navigation(&mut runtime, &queued_tasks, &recorded_events);

    assert_eq!(runtime.tabs.len(), 1);
    assert_eq!(
        runtime.tabs[0].project_root,
        dunce::canonicalize(&workspace_home).unwrap()
    );
    assert_eq!(runtime.recent_projects.len(), 1);
    assert_eq!(
        runtime.recent_projects[0].path,
        dunce::canonicalize(&workspace_home).unwrap()
    );
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Hub,
            event: BackendEvent::CloneProjectDone {
                workspace_home: emitted_workspace_home,
            },
            ..
        } if emitted_workspace_home == &workspace_home.display().to_string()
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Project(_),
            event: BackendEvent::WindowCanvasState { .. },
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Project(_),
            event: BackendEvent::PmStatus {
                available: true,
                ..
            },
            ..
        }
    )));
}

#[test]
fn open_project_path_for_worktree_remembers_workspace_home_only() {
    // Issue #2867: open_project_path で worktree path を渡したとき、tab は
    // worktree で開く (direct-pick) が、recent_projects は workspace home
    // (bare repo の親) に正規化されて 1 件だけ残る。同じ workspace の
    // 別 worktree を続けて開いても recent_projects は増えない。
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let workspace_home = temp.path().join("workspace");
    let bare_repo = workspace_home.join("repo.git");
    fs::create_dir_all(&workspace_home).expect("workspace home");
    let output = gwt_core::process::hidden_command("git")
        .args(["init", "--bare", bare_repo.to_str().expect("bare path")])
        .output()
        .expect("git init --bare");
    assert!(
        output.status.success(),
        "git init --bare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bootstrap = workspace_home.join(".bootstrap");
    let clone = gwt_core::process::hidden_command("git")
        .args([
            "clone",
            bare_repo.to_str().unwrap(),
            bootstrap.to_str().unwrap(),
        ])
        .output()
        .expect("git clone");
    assert!(clone.status.success(), "git clone failed");
    for (key, value) in [
        ("user.email", "test@example.com"),
        ("user.name", "Test User"),
    ] {
        let cfg = gwt_core::process::hidden_command("git")
            .args(["config", key, value])
            .current_dir(&bootstrap)
            .output()
            .expect("git config");
        assert!(cfg.status.success(), "git config {key} failed");
    }
    for args in [
        vec!["checkout", "-b", "develop"],
        vec!["commit", "--allow-empty", "-m", "init"],
        vec!["push", "origin", "develop"],
    ] {
        let out = gwt_core::process::hidden_command("git")
            .args(&args)
            .current_dir(&bootstrap)
            .output()
            .expect("git command");
        assert!(out.status.success(), "git {args:?} failed");
    }
    fs::remove_dir_all(&bootstrap).expect("remove bootstrap");

    let develop_worktree = workspace_home.join("develop");
    let feature_worktree = workspace_home.join("feature/alpha");
    for (path, branch_args) in [
        (
            &develop_worktree,
            vec!["worktree", "add", "@PATH@", "develop"],
        ),
        (
            &feature_worktree,
            vec![
                "worktree",
                "add",
                "-b",
                "feature/alpha",
                "@PATH@",
                "develop",
            ],
        ),
    ] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("worktree parent");
        }
        let path_str = path.to_str().unwrap().to_string();
        let resolved: Vec<&str> = branch_args
            .iter()
            .map(|a| {
                if *a == "@PATH@" {
                    path_str.as_str()
                } else {
                    *a
                }
            })
            .collect();
        let out = gwt_core::process::hidden_command("git")
            .args(&resolved)
            .current_dir(&bare_repo)
            .output()
            .expect("git worktree add");
        assert!(
            out.status.success(),
            "git {resolved:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let (mut runtime, recorded_events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let canonical_home = dunce::canonicalize(&workspace_home).unwrap();

    assert!(runtime
        .open_project_path_events(develop_worktree.clone())
        .is_empty());
    let prepared = take_project_navigation_completion(&recorded_events);
    runtime.handle_project_navigation_prepared(prepared);
    assert_eq!(runtime.tabs.len(), 1);
    assert_eq!(
        runtime.tabs[0].project_root,
        dunce::canonicalize(&develop_worktree).unwrap(),
        "tab must open at the chosen worktree (SC-035 direct-pick preserved)"
    );
    assert_eq!(runtime.recent_projects.len(), 1);
    assert_eq!(
        runtime.recent_projects[0].path, canonical_home,
        "recent_projects must collapse to workspace home, not worktree path (Issue #2867)"
    );

    assert!(runtime
        .open_project_path_events(feature_worktree.clone())
        .is_empty());
    let prepared = take_project_navigation_completion(&recorded_events);
    runtime.handle_project_navigation_prepared(prepared);
    assert_eq!(
        runtime.recent_projects.len(),
        1,
        "opening another worktree in the same workspace must not add a new recent entry"
    );
    assert_eq!(
        runtime.recent_projects[0].path, canonical_home,
        "second worktree open must keep workspace home as the canonical recent entry"
    );
}

#[test]
fn clone_project_start_validation_uses_clone_project_error_event() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::CloneProjectStart {
            url: "".to_string(),
            parent_path: "".to_string(),
        },
    );

    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::CloneProjectError { message },
            ..
        } if client_id == "client-1" && message.contains("repository URL")
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            event: BackendEvent::ProjectOpenError { .. },
            ..
        }
    )));
}

#[test]
fn skip_migration_events_clears_pending_flag_without_broadcast() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");

    let tab = migration_pending_tab("tab-1", project);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.skip_migration_events("tab-1");
    assert!(events.is_empty(), "skip must not emit events itself");
    assert!(!runtime.tabs[0].migration_pending);
}

#[test]
fn skip_migration_events_keeps_normal_git_and_redetects_on_next_launch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    init_repo(&project);

    let (mut runtime, recorded_events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;
    runtime.open_project_path_events(project.clone());
    let open_events =
        commit_pending_project_navigation(&mut runtime, &queued_tasks, &recorded_events);
    let tab_id = runtime.active_tab_id.clone().expect("active tab");

    assert!(open_events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Project(key),
            event: BackendEvent::MigrationDetected { tab_id, .. },
            ..
        } if Some(key) == runtime.project_key_for_tab(tab_id)
    )));

    let skip_events = runtime.skip_migration_events(&tab_id);
    assert!(skip_events.is_empty(), "skip must not mutate repository");
    assert!(matches!(
        gwt_git::detect_repo_type(&project),
        gwt_git::RepoType::Normal {
            needs_migration: true,
            ..
        }
    ));

    let (mut next_runtime, next_recorded_events) =
        sample_runtime_with_events(temp.path(), Vec::new(), None);
    let (next_blocking_tasks, next_queued_tasks) = BlockingTaskSpawner::queued();
    next_runtime.blocking_tasks = next_blocking_tasks;
    next_runtime.open_project_path_events(project);
    let next_events = commit_pending_project_navigation(
        &mut next_runtime,
        &next_queued_tasks,
        &next_recorded_events,
    );

    assert!(
        next_events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Project(key),
                event: BackendEvent::MigrationDetected { tab_id, .. },
                ..
            } if Some(key) == next_runtime.project_key_for_tab(tab_id)
        )),
        "skip is launch-local; the modal must be shown again next launch"
    );
}

#[test]
fn quit_migration_events_requests_app_quit_without_repository_changes() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    init_repo(&project);

    let tab = migration_pending_tab("tab-1", project.clone());
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.quit_migration_events("tab-1");

    assert!(
        events.is_empty(),
        "quit is delivered through the event proxy"
    );
    let recorded_events = recorded_events.lock().expect("recorded events");
    assert!(recorded_events
        .iter()
        .any(|event| matches!(recorded_project_payload(event), UserEvent::QuitApp { .. })));
    assert!(matches!(
        gwt_git::detect_repo_type(&project),
        gwt_git::RepoType::Normal {
            needs_migration: true,
            ..
        }
    ));
}

#[test]
fn open_project_with_existing_migration_backup_emits_recovery_error() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    init_repo(&project);
    fs::create_dir_all(project.join(gwt_core::migration::backup::BACKUP_DIR_NAME))
        .expect("migration backup dir");

    let (mut runtime, recorded_events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;
    runtime.open_project_path_events(project.clone());
    let events = commit_pending_project_navigation(&mut runtime, &queued_tasks, &recorded_events);

    assert!(
        events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Project(key),
                event: BackendEvent::MigrationDetected { tab_id, .. },
                ..
            } if Some(key) == runtime.project_key_for_tab(tab_id)
        )),
        "Normal Git layout should still open a migration-pending tab"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Project(key),
                event: BackendEvent::MigrationError {
                    tab_id,
                    phase,
                    recovery,
                    message,
                    ..
                },
                ..
            } if Some(key) == runtime.project_key_for_tab(tab_id) && phase == "backup"
                && recovery == "partial"
                && message.contains(".gwt-migration-backup")
        )),
        "existing migration backup must be surfaced as a recovery error"
    );
}

// SPEC-2041 Phase 19 (FR-065 / CodeRabbit review on PR #2630): renderer-
// supplied log paths must canonicalize into the gwt update logs root.
// These tests cover the `validate_update_log_path` pure validator.
#[test]
fn validate_update_log_path_accepts_file_inside_logs_root() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    let _gwt_home = ScopedGwtHome::set(logs_root.path());
    let log_file = logs_root.path().join("update-2026-05-10.log");
    std::fs::write(&log_file, b"{}\n").unwrap();

    let resolved =
        super::super::validate_update_log_path(log_file.to_str().unwrap(), logs_root.path());
    assert!(
        resolved.is_some(),
        "expected file inside logs_root to validate"
    );
    let resolved = resolved.unwrap();
    assert!(resolved.is_absolute());
    assert!(resolved.ends_with("update-2026-05-10.log"));
}

#[test]
fn validate_update_log_path_rejects_files_outside_logs_root() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    let _gwt_home = ScopedGwtHome::set(logs_root.path());
    let outside = tempfile::tempdir().expect("outside tempdir");
    let outside_file = outside.path().join("evil.txt");
    std::fs::write(&outside_file, b"steal me").unwrap();

    let resolved =
        super::super::validate_update_log_path(outside_file.to_str().unwrap(), logs_root.path());
    assert!(resolved.is_none(), "outside-root paths must be rejected");
}

#[test]
fn validate_update_log_path_rejects_url_schemes_and_empty() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    let _gwt_home = ScopedGwtHome::set(logs_root.path());
    for raw in [
        "",
        "   ",
        "http://evil.example/log",
        "https://evil.example/log",
        "file:///etc/passwd",
    ] {
        assert!(
            super::super::validate_update_log_path(raw, logs_root.path()).is_none(),
            "expected `{raw}` to be rejected",
        );
    }
}

#[test]
fn validate_update_log_path_rejects_directories() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    let _gwt_home = ScopedGwtHome::set(logs_root.path());
    // Caller passes the logs root itself; a directory must not be opened
    // as a file.
    let resolved = super::super::validate_update_log_path(
        logs_root.path().to_str().unwrap(),
        logs_root.path(),
    );
    assert!(resolved.is_none(), "directories must be rejected");
}

#[test]
fn validate_update_log_path_rejects_missing_files() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    let _gwt_home = ScopedGwtHome::set(logs_root.path());
    let missing = logs_root.path().join("does-not-exist.log");
    let resolved =
        super::super::validate_update_log_path(missing.to_str().unwrap(), logs_root.path());
    assert!(resolved.is_none(), "missing files must be rejected");
}

// SPEC-2785 FR-E: open_server_url requests must be gated by an exact
// same-origin match against the embedded server's bound URL. The shared
// validator function is reused by `AppRuntime::open_server_url_events`
// so a mismatched origin cannot smuggle an arbitrary URL into the OS
// opener.
#[test]
fn validate_server_url_accepts_exact_bound_url() {
    let allowed = Some("http://127.0.0.1:54321/");
    assert!(super::super::validate_server_url(
        allowed,
        "http://127.0.0.1:54321/"
    ));
}

#[test]
fn validate_server_url_rejects_different_port() {
    let allowed = Some("http://127.0.0.1:54321/");
    assert!(!super::super::validate_server_url(
        allowed,
        "http://127.0.0.1:54322/"
    ));
}

#[test]
fn validate_server_url_rejects_different_scheme() {
    let allowed = Some("http://127.0.0.1:54321/");
    assert!(!super::super::validate_server_url(
        allowed,
        "https://127.0.0.1:54321/"
    ));
}

#[test]
fn validate_server_url_rejects_when_allowed_is_none() {
    assert!(!super::super::validate_server_url(
        None,
        "http://127.0.0.1:54321/"
    ));
}

#[test]
fn validate_server_url_rejects_external_origin() {
    let allowed = Some("http://127.0.0.1:54321/");
    assert!(!super::super::validate_server_url(
        allowed,
        "http://evil.example/"
    ));
}

// SPEC-2785 SC-4: `open_server_url_events` returns an empty event list and
// performs no OS opener side effect when the requested URL does not match
// the configured server URL. The state mutation guard is `server_url`
// being None or unequal to the request.
#[test]
fn open_server_url_events_rejects_mismatched_origin() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    runtime.set_server_url("http://127.0.0.1:54321/".to_string());
    let outbound = runtime.open_server_url_events("client-1", "http://evil.example/".to_string());
    assert!(
        outbound.is_empty(),
        "mismatched origin must yield no outbound events"
    );
}

#[test]
fn open_server_url_events_rejects_when_server_url_unset() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (runtime, _events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let outbound =
        runtime.open_server_url_events("client-1", "http://127.0.0.1:54321/".to_string());
    assert!(
        outbound.is_empty(),
        "unset server URL must reject any open request"
    );
}

#[test]
fn codex_hook_discovery_mode_keeps_worktree_local_hooks_for_every_codex_version() {
    use gwt_skills::CodexHookDiscoveryMode;

    assert_eq!(
        super::super::codex_hook_discovery_mode_from_detected_codex_version(Some("0.130.0")),
        Some(CodexHookDiscoveryMode::WorktreeLocal)
    );
    assert_eq!(
        super::super::codex_hook_discovery_mode_from_detected_codex_version(Some(
            "0.131.0-alpha.9"
        )),
        Some(CodexHookDiscoveryMode::WorktreeLocal)
    );
    assert_eq!(
        super::super::codex_hook_discovery_mode_from_detected_codex_version(Some(
            "0.131.0-alpha.21"
        )),
        Some(CodexHookDiscoveryMode::Both)
    );
    assert_eq!(
        super::super::codex_hook_discovery_mode_from_detected_codex_version(Some("0.131.0")),
        Some(CodexHookDiscoveryMode::Both)
    );
    // Legacy selector strings are not measured version evidence.
    assert_eq!(
        super::super::codex_hook_discovery_mode_from_detected_codex_version(Some("latest")),
        None
    );
    assert_eq!(
        super::super::codex_hook_discovery_mode_from_detected_codex_version(Some("installed")),
        None
    );
}

/// A detected version on the launch config determines hook compatibility;
/// non-Codex agents retain their unconditional mode.
#[test]
fn codex_detected_version_and_other_agents_keep_their_existing_modes() {
    use gwt_skills::CodexHookDiscoveryMode;

    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut detected = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(temp.path())
        .build();
    detected.tool_version = Some("0.130.0".to_string());
    let stale_evidence = gwt_agent::HostRunnerHealthReport {
        version_output: Some("0.133.0".to_string()),
    };
    assert_eq!(
        super::super::codex_hook_discovery_mode_for_launch_config(&detected, Some(&stale_evidence)),
        CodexHookDiscoveryMode::WorktreeLocal,
    );

    let claude = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
        .working_dir(temp.path())
        .build();
    assert_eq!(
        super::super::codex_hook_discovery_mode_for_launch_config(&claude, None),
        CodexHookDiscoveryMode::WorkspaceHome,
    );
}

#[test]
fn codex_hook_discovery_mode_extracts_installed_codex_version_output() {
    use gwt_skills::CodexHookDiscoveryMode;

    assert_eq!(
        super::super::codex_hook_discovery_mode_from_codex_version_output("codex-cli 0.133.0\n"),
        Some(CodexHookDiscoveryMode::Both)
    );
    assert_eq!(
        super::super::codex_hook_discovery_mode_from_codex_version_output("codex 0.130.0\n"),
        Some(CodexHookDiscoveryMode::WorktreeLocal)
    );
    assert_eq!(
        super::super::codex_hook_discovery_mode_from_codex_version_output("unexpected output\n"),
        None
    );
}

#[test]
fn codex_hook_discovery_mode_reuses_canonical_health_evidence() {
    use gwt_skills::CodexHookDiscoveryMode;

    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(temp.path())
        .build();
    let old = gwt_agent::HostRunnerHealthReport {
        version_output: Some("codex-cli 0.130.0".to_string()),
    };
    let current = gwt_agent::HostRunnerHealthReport {
        version_output: Some("codex-cli 0.133.0".to_string()),
    };
    let unknown = gwt_agent::HostRunnerHealthReport {
        version_output: Some("unexpected output".to_string()),
    };

    assert_eq!(
        super::super::codex_hook_discovery_mode_for_launch_config(&config, Some(&old)),
        CodexHookDiscoveryMode::WorktreeLocal,
    );
    assert_eq!(
        super::super::codex_hook_discovery_mode_for_launch_config(&config, Some(&current)),
        CodexHookDiscoveryMode::Both,
    );
    assert_eq!(
        super::super::codex_hook_discovery_mode_for_launch_config(&config, Some(&unknown)),
        CodexHookDiscoveryMode::Both,
    );
}

#[test]
fn docker_codex_hook_discovery_mode_keeps_safe_both_fallback() {
    use gwt_skills::CodexHookDiscoveryMode;

    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(temp.path())
        .build();
    config.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;

    assert_eq!(
        super::super::codex_hook_discovery_mode_for_launch_config(&config, None),
        CodexHookDiscoveryMode::Both,
    );
}

/// Issue #5194 AC-1: Codex 0.160 does not read the workspace-home hooks file
/// from a linked worktree, so a fresh worktree must always receive its own
/// `.codex/hooks.json` or SessionStart never reaches gwt.
#[test]
fn host_codex_launch_writes_worktree_local_hooks_into_a_new_linked_worktree() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path().join("home"));
    let repo = temp.path().join("repo");
    let gitdir = repo.join("repo.git/worktrees/issue-1");
    let worktree = repo.join("work/issue-1");
    fs::create_dir_all(&gitdir).expect("gitdir");
    fs::create_dir_all(&worktree).expect("worktree");
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", gitdir.display()),
    )
    .expect("write .git");

    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&worktree)
        .build();
    let report = gwt_agent::HostRunnerHealthReport {
        version_output: Some("codex-cli 0.160.0".to_string()),
    };
    let mode = super::super::codex_hook_discovery_mode_for_launch_config(&config, Some(&report));
    gwt_skills::generate_codex_hooks_for_mode(&worktree, mode).expect("generate hooks");

    let local = fs::read_to_string(worktree.join(".codex/hooks.json"))
        .expect("new linked worktree must receive a worktree-local .codex/hooks.json");
    assert!(local.contains("SessionStart"), "{local}");
}

#[test]
fn codex_hook_discovery_has_no_standalone_process_probe() {
    let source = include_str!("../launch.rs");
    let discovery = source
        .split("pub(super) fn codex_hook_discovery_mode_for_launch_config")
        .nth(1)
        .and_then(|tail| tail.split("pub(super) fn maybe_register_codex").next())
        .expect("Codex hook discovery implementation");

    assert!(!discovery.contains("detect_installed_codex_hook_discovery_mode"));
    assert!(!discovery.contains(".output()"));
}

#[test]
fn codex_hook_trust_launch_enabled_registers_host_codex_hooks() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let codex_home = home.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex home");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let mut settings = Settings::default();
    settings.agent.codex_trust_managed_hooks = Some(true);
    settings.save(&profile_config_path).unwrap();

    let worktree = tempdir().expect("worktree tempdir");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();
    let mut launch_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.path())
        .build();
    launch_config
        .env_vars
        .insert("CODEX_HOME".to_string(), codex_home.display().to_string());

    let report = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &launch_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        None,
    )
    .unwrap()
    .expect("enabled host Codex launch should register trust");

    assert_eq!(report.trusted_entries.len(), 5);
    let codex_config_path = gwt_core::paths::normalize_windows_child_process_path(
        &fs::canonicalize(&codex_home).unwrap(),
    )
    .join("config.toml");
    let config = fs::read_to_string(&codex_config_path).unwrap();
    assert!(
        config.contains("trusted_hash"),
        "Codex config should contain trusted hashes, got: {config}"
    );
    assert_eq!(report.config_path, codex_config_path);
}

/// Issue #3967 (recurrence in v9.93.1): materialization resolves the fallback
/// binary a managed hook command embeds, pins it for the duration of the
/// generation call, and releases the pin on the way out. Trust pre-registration
/// then re-derived an answer of its own, and for a gwt started from a checkout
/// build that answer was `target/debug/gwtd` — reduced to the bare `gwtd` for a
/// config outside that checkout — where the generated command carried the
/// installed absolute path. All five managed hooks failed the exact-command
/// match, and Codex stopped every launch on `Hooks need review`. The launch has
/// to vouch for the value materialization actually wrote.
#[test]
fn codex_hook_trust_launch_vouches_for_the_binary_materialization_pinned() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let codex_home = home.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex home");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");

    // An installed binary the ambient resolver cannot reach: it is neither this
    // process, nor its sibling, nor anything on PATH. Materialization pins it,
    // generates with it, and drops the pin — the shape of the `GWT_HOOK_BIN`
    // guard in `regenerate_managed_hook_configs_for_targets`.
    let generated_hook_bin = home
        .path()
        .join("Programs")
        .join("GWT")
        .join("gwtd")
        .to_string_lossy()
        .into_owned();
    {
        let _pin = gwt_skills::settings_local::ScopedHookBin::set(&generated_hook_bin);
        gwt_skills::generate_codex_hooks(worktree.path()).unwrap();
    }

    // Re-deriving the binary once the pin is gone is what the launch used to
    // do, and it cannot reach the pinned value — every managed hook stays
    // untrusted and Codex stops the launch.
    let guessed = gwt_skills::register_codex_managed_hook_trust_for_mode(
        worktree.path(),
        &home.path().join("guessed-codex-config.toml"),
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    )
    .unwrap();
    assert!(
        !guessed.untrusted_gwt_hooks.is_empty(),
        "a re-derived binary must not be able to vouch for a pin it cannot reach; if it can, \
         the launch no longer needs to be told which binary was written: {guessed:?}"
    );

    let mut launch_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.path())
        .build();
    launch_config
        .env_vars
        .insert("CODEX_HOME".to_string(), codex_home.display().to_string());

    let report = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &launch_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        Some(generated_hook_bin.as_str()),
    )
    .unwrap()
    .expect("enabled host Codex launch should register trust");

    assert!(
        report.untrusted_gwt_hooks.is_empty(),
        "every hook materialization generated must be trusted, got: {report:?}"
    );
    assert_eq!(report.trusted_entries.len(), 5);
}

#[test]
fn codex_project_trust_launch_registers_the_process_stable_host_worktree() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let profile_config_path = home.path().join(".gwt/config.toml");
    let mut settings = Settings::default();
    settings.agent.codex_trust_managed_hooks = Some(false);
    settings
        .save(&profile_config_path)
        .expect("save hook trust opt-out");
    let worktree = tempdir().expect("worktree tempdir");
    let codex_home = tempdir().expect("codex home");

    let worktrees = vec![gwt_git::WorktreeInfo {
        path: worktree.path().to_path_buf(),
        branch: Some("work/issue-42".to_string()),
        locked: false,
        prunable: false,
    }];
    let mut managed = None;
    for mode in [
        gwt_agent::SessionMode::Normal,
        gwt_agent::SessionMode::Resume,
    ] {
        let candidate = super::super::IssueMonitorTrustCandidate {
            issue_number: 42,
            project_root: home.path().to_path_buf(),
            session_mode: mode,
        };
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .working_dir(worktree.path())
            .branch("work/issue-42")
            .linked_issue_number(42)
            .session_mode(mode)
            .build();
        managed = super::super::validate_issue_monitor_managed_codex_worktree(
            Some(&candidate),
            home.path(),
            &config,
            &worktrees,
        )
        .expect("exact Issue Monitor worktree must validate");
        assert!(managed.is_some(), "{mode:?} launch must mint managed proof");
    }
    let managed = managed.expect("Codex Issue Monitor launch returns managed proof");

    let mut trust_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.path())
        .session_mode(gwt_agent::SessionMode::Resume)
        .build();
    trust_config.env_vars.insert(
        "CODEX_HOME".to_string(),
        codex_home.path().display().to_string(),
    );
    let report =
        super::super::register_codex_managed_project_trust_for_resolved_launch_with_host_context(
            &profile_config_path,
            &managed,
            &trust_config,
            None,
            Some(codex_home.path().as_os_str()),
            Some(home.path()),
        )
        .expect("managed Codex launch must register project trust")
        .expect("host launch returns its project trust report");
    assert_eq!(
        Settings::load_from_path(&profile_config_path)
            .expect("reload settings")
            .agent
            .codex_trust_managed_hooks,
        Some(false),
        "directory trust must not alter or depend on the managed-hook opt-out"
    );

    let canonical_worktree = gwt_core::paths::normalize_windows_child_process_path(
        &fs::canonicalize(worktree.path()).unwrap(),
    );
    assert_eq!(report.project_path, canonical_worktree);
    assert_eq!(
        report.config_path,
        gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(codex_home.path()).unwrap(),
        )
        .join("config.toml")
    );
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(&report.config_path).unwrap()).unwrap();
    assert_eq!(
        config["projects"][canonical_worktree.to_string_lossy().as_ref()]["trust_level"].as_str(),
        Some("trusted")
    );
    assert!(
        !home.path().join(".codex/config.toml").exists(),
        "project trust must use the effective CODEX_HOME"
    );
}

#[test]
fn host_codex_config_path_matches_the_final_child_environment_and_cwd() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let child_cwd = temp.path().join("worktree");
    fs::create_dir_all(&child_cwd).expect("create child cwd");
    let relative_codex_home = child_cwd.join("relative/codex-home");
    fs::create_dir_all(&relative_codex_home).expect("create relative CODEX_HOME");
    let canonical_codex_home = gwt_core::paths::normalize_windows_child_process_path(
        &fs::canonicalize(&relative_codex_home).unwrap(),
    );
    let os_user_home = temp.path().join("os-user-home");
    fs::create_dir_all(&os_user_home).expect("create OS user home");

    let relative_codex_home_env =
        HashMap::from([("CODEX_HOME".to_string(), "relative/codex-home".to_string())]);
    assert_eq!(
        super::super::effective_host_codex_config_path(
            &child_cwd,
            &relative_codex_home_env,
            super::super::HostEnvKeySemantics::CaseSensitive,
            Some(&os_user_home),
        )
        .expect("relative CODEX_HOME"),
        canonical_codex_home.join("config.toml"),
        "existing relative CODEX_HOME must canonicalize from the final child cwd"
    );

    let unix_home_path = temp.path().join("unix-home");
    fs::create_dir_all(&unix_home_path).expect("create Unix HOME");
    let unix_home = HashMap::from([("HOME".to_string(), unix_home_path.display().to_string())]);
    assert_eq!(
        super::super::effective_host_codex_config_path(
            &child_cwd,
            &unix_home,
            super::super::HostEnvKeySemantics::CaseSensitive,
            Some(&os_user_home),
        )
        .expect("Unix HOME fallback"),
        unix_home_path.join(".codex/config.toml")
    );

    let windows_env = HashMap::from([
        ("HOME".to_string(), "ignored/home".to_string()),
        (
            "userprofile".to_string(),
            "ignored/windows-profile".to_string(),
        ),
    ]);
    assert_eq!(
        super::super::effective_host_codex_config_path(
            &child_cwd,
            &windows_env,
            super::super::HostEnvKeySemantics::WindowsCaseInsensitive,
            Some(&os_user_home),
        )
        .expect("Windows OS user-home fallback"),
        os_user_home.join(".codex/config.toml"),
        "Codex/dirs 6 on Windows ignores HOME and USERPROFILE env overrides"
    );
}

#[test]
fn host_codex_config_path_rejects_relative_home_and_missing_relative_codex_home() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let child_cwd = temp.path().join("worktree");
    fs::create_dir_all(&child_cwd).expect("create child cwd");
    let os_user_home = temp.path().join("os-user-home");

    let relative_home = HashMap::from([("HOME".to_string(), "relative/home".to_string())]);
    let error = super::super::effective_host_codex_config_path(
        &child_cwd,
        &relative_home,
        super::super::HostEnvKeySemantics::CaseSensitive,
        Some(&os_user_home),
    )
    .expect_err("Codex requires its Unix fallback home to be absolute");
    assert!(error.contains("HOME must be absolute"), "{error}");

    let missing_codex_home =
        HashMap::from([("CODEX_HOME".to_string(), "missing/codex-home".to_string())]);
    let error = super::super::effective_host_codex_config_path(
        &child_cwd,
        &missing_codex_home,
        super::super::HostEnvKeySemantics::CaseSensitive,
        Some(&os_user_home),
    )
    .expect_err("Codex metadata-checks CODEX_HOME before canonicalizing it");
    assert!(error.contains("CODEX_HOME"), "{error}");

    let error = super::super::effective_host_codex_config_path(
        &child_cwd,
        &HashMap::new(),
        super::super::HostEnvKeySemantics::CaseSensitive,
        Some(&os_user_home),
    )
    .expect_err("Unix Codex fallback must not borrow the parent process home");
    assert!(error.contains("HOME"), "{error}");
}

#[test]
fn host_codex_config_path_rejects_windows_duplicate_case_ambiguity() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let child_cwd = temp.path().join("worktree");
    fs::create_dir_all(&child_cwd).expect("create child cwd");
    let ambiguous_env = HashMap::from([
        ("CODEX_HOME".to_string(), "first".to_string()),
        ("codex_home".to_string(), "second".to_string()),
    ]);

    let error = super::super::effective_host_codex_config_path(
        &child_cwd,
        &ambiguous_env,
        super::super::HostEnvKeySemantics::WindowsCaseInsensitive,
        Some(temp.path()),
    )
    .expect_err("case-insensitive duplicate CODEX_HOME values must fail closed");
    assert!(error.contains("ambiguous CODEX_HOME"), "{error}");
}

#[test]
fn managed_project_trust_skips_worktree_relative_codex_home() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&project).expect("create project");
    fs::create_dir_all(&worktree).expect("create worktree");
    let profile_config_path = project.join(".gwt/config.toml");
    let candidate = super::super::IssueMonitorTrustCandidate {
        issue_number: 42,
        project_root: project.clone(),
        session_mode: gwt_agent::SessionMode::Normal,
    };
    let worktrees = vec![gwt_git::WorktreeInfo {
        path: worktree.clone(),
        branch: Some("work/issue-42".to_string()),
        locked: false,
        prunable: false,
    }];
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&worktree)
        .branch("work/issue-42")
        .linked_issue_number(42)
        .session_mode(gwt_agent::SessionMode::Normal)
        .build();
    config
        .env_vars
        .insert("CODEX_HOME".to_string(), "relative/codex-home".to_string());
    fs::create_dir_all(worktree.join("relative/codex-home")).expect("create relative CODEX_HOME");
    let managed = super::super::validate_issue_monitor_managed_codex_worktree(
        Some(&candidate),
        &project,
        &config,
        &worktrees,
    )
    .expect("managed provenance validation")
    .expect("managed proof");

    let report =
        super::super::register_codex_managed_project_trust_for_resolved_launch_with_host_context(
            &profile_config_path,
            &managed,
            &config,
            None,
            None,
            Some(temp.path()),
        )
        .expect("worktree-relative CODEX_HOME is an explicit no-write boundary");

    assert!(report.is_none());
    assert!(
        !worktree.join("relative/codex-home/config.toml").exists(),
        "project trust must not be written to a worktree-local CODEX_HOME"
    );
}

#[test]
fn managed_project_trust_skips_absolute_process_codex_home_inside_worktree() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let worktree = temp.path().join("worktree");
    let nested_codex_home = worktree.join(".codex");
    fs::create_dir_all(&project).expect("create project");
    fs::create_dir_all(&nested_codex_home).expect("create nested Codex home");
    let profile_config_path = project.join(".gwt/config.toml");
    let candidate = super::super::IssueMonitorTrustCandidate {
        issue_number: 42,
        project_root: project.clone(),
        session_mode: gwt_agent::SessionMode::Normal,
    };
    let worktrees = vec![gwt_git::WorktreeInfo {
        path: worktree.clone(),
        branch: Some("work/issue-42".to_string()),
        locked: false,
        prunable: false,
    }];
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&worktree)
        .branch("work/issue-42")
        .linked_issue_number(42)
        .session_mode(gwt_agent::SessionMode::Normal)
        .build();
    config.env_vars.insert(
        "CODEX_HOME".to_string(),
        nested_codex_home.display().to_string(),
    );
    let managed = super::super::validate_issue_monitor_managed_codex_worktree(
        Some(&candidate),
        &project,
        &config,
        &worktrees,
    )
    .expect("managed provenance validation")
    .expect("managed proof");

    let report =
        super::super::register_codex_managed_project_trust_for_resolved_launch_with_host_context(
            &profile_config_path,
            &managed,
            &config,
            None,
            Some(nested_codex_home.as_os_str()),
            Some(temp.path()),
        )
        .expect("worktree-contained process CODEX_HOME is an explicit no-write boundary");

    assert!(report.is_none());
    assert!(
        !nested_codex_home.join("config.toml").exists(),
        "project trust must not be written inside the managed worktree"
    );
}

#[test]
fn managed_project_trust_skips_profile_supplied_arbitrary_codex_home() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let worktree = temp.path().join("worktree");
    let default_home = temp.path().join("default-home");
    let profile_codex_home = temp.path().join("profile-codex-home");
    fs::create_dir_all(&project).expect("create project");
    fs::create_dir_all(&worktree).expect("create worktree");
    fs::create_dir_all(&default_home).expect("create default home");
    fs::create_dir_all(&profile_codex_home).expect("create profile Codex home");
    let profile_config_path = project.join(".gwt/config.toml");
    let candidate = super::super::IssueMonitorTrustCandidate {
        issue_number: 42,
        project_root: project.clone(),
        session_mode: gwt_agent::SessionMode::Normal,
    };
    let worktrees = vec![gwt_git::WorktreeInfo {
        path: worktree.clone(),
        branch: Some("work/issue-42".to_string()),
        locked: false,
        prunable: false,
    }];
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&worktree)
        .branch("work/issue-42")
        .linked_issue_number(42)
        .session_mode(gwt_agent::SessionMode::Normal)
        .build();
    config.env_vars.insert(
        "CODEX_HOME".to_string(),
        profile_codex_home.display().to_string(),
    );
    let managed = super::super::validate_issue_monitor_managed_codex_worktree(
        Some(&candidate),
        &project,
        &config,
        &worktrees,
    )
    .expect("managed provenance validation")
    .expect("managed proof");

    let report =
        super::super::register_codex_managed_project_trust_for_resolved_launch_with_host_context(
            &profile_config_path,
            &managed,
            &config,
            None,
            None,
            Some(&default_home),
        )
        .expect("custom CODEX_HOME is an explicit no-write boundary");

    assert!(report.is_none());
    assert!(
        !profile_codex_home.join("config.toml").exists(),
        "gwt must not create project trust in a profile-owned arbitrary CODEX_HOME"
    );
    assert!(
        !default_home.join(".codex/config.toml").exists(),
        "a mismatched child config must not cause a useless default-config trust write"
    );
}

#[test]
fn codex_project_trust_launch_is_codex_only_and_fail_closed() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");

    let candidate = super::super::IssueMonitorTrustCandidate {
        issue_number: 42,
        project_root: home.path().to_path_buf(),
        session_mode: gwt_agent::SessionMode::Normal,
    };
    let worktrees = vec![gwt_git::WorktreeInfo {
        path: worktree.path().to_path_buf(),
        branch: Some("work/issue-42".to_string()),
        locked: false,
        prunable: false,
    }];
    let claude_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
        .working_dir(worktree.path())
        .branch("work/issue-42")
        .linked_issue_number(42)
        .build();
    let claude = super::super::validate_issue_monitor_managed_codex_worktree(
        Some(&candidate),
        home.path(),
        &claude_config,
        &worktrees,
    )
    .expect("non-Codex launch should not fail");
    assert!(claude.is_none());
    assert!(!home.path().join(".codex/config.toml").exists());

    let codex_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.path())
        .branch("work/issue-42")
        .linked_issue_number(42)
        .build();
    for session_mode in [
        gwt_agent::SessionMode::Normal,
        gwt_agent::SessionMode::Continue,
    ] {
        let mut unowned_config = codex_config.clone();
        unowned_config.session_mode = session_mode;
        let unowned = super::super::validate_issue_monitor_managed_codex_worktree(
            None,
            home.path(),
            &unowned_config,
            &worktrees,
        )
        .expect("unowned Codex launch should not fail");
        assert!(
            unowned.is_none(),
            "manual, Quick Start, and generic Continue launches must never mint managed trust proof ({session_mode:?})"
        );
    }
    let managed = super::super::validate_issue_monitor_managed_codex_worktree(
        Some(&candidate),
        home.path(),
        &codex_config,
        &worktrees,
    )
    .unwrap()
    .unwrap();
    let invalid_codex_home = home.path().join("not-a-directory");
    fs::write(&invalid_codex_home, "file").unwrap();
    let mut invalid_config = codex_config;
    invalid_config.env_vars.insert(
        "CODEX_HOME".to_string(),
        invalid_codex_home.display().to_string(),
    );
    let error =
        super::super::register_codex_managed_project_trust_for_resolved_launch_with_host_context(
            &profile_config_path,
            &managed,
            &invalid_config,
            None,
            Some(invalid_codex_home.as_os_str()),
            Some(home.path()),
        )
        .expect_err("project trust failure must abort before Codex can prompt");
    assert!(error.contains("failed to trust gwt-managed Codex worktree"));
}

#[test]
fn issue_monitor_project_trust_candidate_requires_complete_feedback_provenance() {
    let project = tempdir().expect("project tempdir");
    let _gwt_home = ScopedGwtHome::set(project.path());
    let complete = LaunchFeedbackContext {
        client_id: "client-1".to_string(),
        title: "Issue Monitor".to_string(),
        issue_monitor_issue_number: Some(42),
        issue_monitor_delivery_id: Some("delivery-42".to_string()),
        issue_monitor_project_root: Some(project.path().to_path_buf()),
        issue_monitor_session_mode: Some(gwt_agent::SessionMode::Normal),
        issue_monitor_autonomous_handoff: None,
        issue_monitor_autonomous_submit_started: false,
        issue_monitor_review_dispatch: false,
    };

    let candidate = super::super::issue_monitor_trust_candidate_from_feedback(
        &gwt_agent::AgentId::Codex,
        Some(&complete),
    )
    .expect("complete provenance")
    .expect("Issue Monitor candidate");
    assert_eq!(candidate.issue_number, 42);
    assert_eq!(candidate.project_root, project.path());
    assert_eq!(candidate.session_mode, gwt_agent::SessionMode::Normal);

    let generic_continue = LaunchFeedbackContext {
        issue_monitor_session_mode: Some(gwt_agent::SessionMode::Continue),
        ..complete.clone()
    };
    let error = super::super::issue_monitor_trust_candidate_from_feedback(
        &gwt_agent::AgentId::Codex,
        Some(&generic_continue),
    )
    .expect_err("generic Continue provenance must never mint managed trust eligibility");
    assert!(error.contains("Normal or Resume"), "{error}");

    let missing_mode = LaunchFeedbackContext {
        issue_monitor_session_mode: None,
        ..complete.clone()
    };
    let error = super::super::issue_monitor_trust_candidate_from_feedback(
        &gwt_agent::AgentId::Codex,
        Some(&missing_mode),
    )
    .expect_err("Issue Monitor ownership without a typed session mode must fail closed");
    assert!(error.contains("session mode provenance"), "{error}");

    let incomplete = LaunchFeedbackContext {
        issue_monitor_project_root: None,
        ..complete
    };
    let error = super::super::issue_monitor_trust_candidate_from_feedback(
        &gwt_agent::AgentId::Codex,
        Some(&incomplete),
    )
    .expect_err("Issue Monitor ownership without project root must fail closed");
    assert!(error.contains("no project root provenance"), "{error}");
    assert!(super::super::issue_monitor_trust_candidate_from_feedback(
        &gwt_agent::AgentId::ClaudeCode,
        Some(&incomplete),
    )
    .expect("non-Codex launch must ignore trust provenance")
    .is_none());
}

#[test]
fn issue_monitor_codex_trust_preflight_failure_keeps_actual_delivery_provenance() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&project).expect("create project");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo_without_origin(&project);
    let launch_effect_id = "phase85-trust-preflight";
    let delivery_id = format!("launch:{launch_effect_id}");
    let feedback = LaunchFeedbackContext {
        client_id: "__issue_monitor__".to_string(),
        title: "Issue Monitor".to_string(),
        issue_monitor_issue_number: Some(42),
        issue_monitor_delivery_id: Some(delivery_id.clone()),
        issue_monitor_project_root: Some(project.clone()),
        issue_monitor_session_mode: Some(gwt_agent::SessionMode::Resume),
        issue_monitor_autonomous_handoff: None,
        issue_monitor_autonomous_submit_started: false,
        issue_monitor_review_dispatch: false,
    };
    let candidate = super::super::issue_monitor_trust_candidate_from_feedback(
        &gwt_agent::AgentId::Codex,
        Some(&feedback),
    )
    .expect("typed Issue Monitor feedback")
    .expect("managed trust candidate");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&worktree)
        .branch("work/issue-42")
        .linked_issue_number(42)
        .session_mode(gwt_agent::SessionMode::Resume)
        .build();
    let invalid_codex_home = temp.path().join("codex-home-is-a-file");
    fs::write(&invalid_codex_home, "not a directory").expect("write invalid CODEX_HOME");
    config.env_vars.insert(
        "CODEX_HOME".to_string(),
        invalid_codex_home.display().to_string(),
    );
    let managed = super::super::validate_issue_monitor_managed_codex_worktree(
        Some(&candidate),
        &project,
        &config,
        &[gwt_git::WorktreeInfo {
            path: worktree,
            branch: Some("work/issue-42".to_string()),
            locked: false,
            prunable: false,
        }],
    )
    .expect("actual feedback must pass provenance preflight")
    .expect("managed worktree proof");
    let error =
        super::super::register_codex_managed_project_trust_for_resolved_launch_with_host_context(
            &project.join(".gwt/config.toml"),
            &managed,
            &config,
            None,
            Some(invalid_codex_home.as_os_str()),
            Some(temp.path()),
        )
        .expect_err("trust writer failure must abort launch preflight");

    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-1",
            "Project",
            project.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab-1"),
    );
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[42], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: 42,
        title: "Codex trust preflight failure".to_string(),
        labels: Vec::new(),
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        42,
        "claim-phase85-trust-preflight",
        "host/session",
        launch_effect_id,
        "2026-08-29T00:00:00Z",
    ));
    assert!(monitor.claim_launch_delivery(
        42,
        &delivery_id,
        &runtime.issue_monitor_materializer_id,
        std::process::id(),
        "tab-1::agent-1",
        |_| false,
    ));
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&project),
        &monitor.prefs(),
    )
    .expect("seed durable delivery");
    let events = runtime.issue_monitor_launch_failed_delivery_events_with_mode(
        Some(&project),
        42,
        &error,
        feedback.issue_monitor_delivery_id.as_deref(),
        feedback.issue_monitor_session_mode.unwrap(),
    );

    assert!(matches!(
        runtime.issue_monitor_launch_deliveries.get(&delivery_id),
        Some(super::super::IssueMonitorLaunchDeliveryState::LaunchFailed {
            message,
            session_mode: gwt_agent::SessionMode::Resume,
        }) if message == &error
    ));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorLaunchFailed {
            issue_number: 42,
            message,
        } if message == &error
    )));
    let persisted =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&project))
            .expect("reload trust-preflight failure");
    assert!(persisted.launching_issues.is_empty());
    assert!(persisted.launched_issues.is_empty());
    assert!(persisted.pending_launch_deliveries.is_empty());
    assert!(persisted
        .failed_issues
        .iter()
        .any(|failure| failure.issue_number == 42 && failure.message == error));
    let restored =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), persisted);
    assert_eq!(
        restored.active_count(),
        0,
        "a fail-closed trust preflight must durably release its Issue Monitor slot"
    );
}

#[test]
fn issue_monitor_codex_project_trust_precedes_runner_probe_and_session_creation() {
    let source = include_str!("../launch.rs");
    let worker = source
        .split("fn spawn_agent_window_async_with_claim")
        .nth(1)
        .expect("launch worker source");
    let validate = worker
        .find("validate_issue_monitor_managed_codex_worktree")
        .expect("managed provenance validation");
    let docker_prepare = worker
        .find("prepare_docker_runtime_for_launch")
        .expect("Docker service and immutable binding preparation");
    let register = worker
        .find("register_codex_managed_project_trust_for_resolved_launch")
        .expect("project trust registration");
    let docker_runner_probe = worker
        .find("resolve_docker_agent_program_with_binding")
        .expect("Docker agent runner probe");
    let runner_probe = worker
        .find("resolve_host_runner_health_checked")
        .expect("host runner probe");
    let session = worker
        .find("gwt_agent::Session::new")
        .expect("durable Session creation");

    assert!(
        validate < register,
        "provenance must be proven before trust"
    );
    assert!(
        docker_prepare < register,
        "Docker identity and service must be bound before container-local trust"
    );
    assert!(
        register < docker_runner_probe,
        "directory trust must be registered before any Docker agent runner probe"
    );
    assert!(
        register < runner_probe,
        "directory trust must be registered before any Codex runner probe"
    );
    assert!(
        register < session,
        "directory trust failure must abort before Session/process materialization"
    );
}

#[test]
fn codex_project_trust_scope_refuses_arbitrary_working_dir_even_with_issue_metadata() {
    let project = tempdir().expect("project tempdir");
    let _gwt_home = ScopedGwtHome::set(project.path());
    let managed_worktree = tempdir().expect("managed worktree tempdir");
    let arbitrary = tempdir().expect("arbitrary worktree tempdir");
    let candidate = super::super::IssueMonitorTrustCandidate {
        issue_number: 42,
        project_root: project.path().to_path_buf(),
        session_mode: gwt_agent::SessionMode::Normal,
    };
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(arbitrary.path())
        .branch("work/issue-42")
        .linked_issue_number(42)
        .build();
    let worktrees = vec![gwt_git::WorktreeInfo {
        path: managed_worktree.path().to_path_buf(),
        branch: Some("work/issue-42".to_string()),
        locked: false,
        prunable: false,
    }];

    let error = super::super::validate_issue_monitor_managed_codex_worktree(
        Some(&candidate),
        project.path(),
        &config,
        &worktrees,
    )
    .expect_err("inventory-external directory must never become trusted");

    assert!(error.contains("authoritative gwt worktree"), "{error}");
}

#[test]
fn codex_project_trust_scope_rejects_mismatched_issue_monitor_provenance() {
    let project = tempdir().expect("project tempdir");
    let _gwt_home = ScopedGwtHome::set(project.path());
    let other_project = tempdir().expect("other project tempdir");
    let worktree = tempdir().expect("worktree tempdir");
    let base_config = || {
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .working_dir(worktree.path())
            .branch("work/issue-42")
            .linked_issue_number(42)
            .build()
    };
    let valid_candidate = super::super::IssueMonitorTrustCandidate {
        issue_number: 42,
        project_root: project.path().to_path_buf(),
        session_mode: gwt_agent::SessionMode::Normal,
    };
    let valid_entry = gwt_git::WorktreeInfo {
        path: worktree.path().to_path_buf(),
        branch: Some("work/issue-42".to_string()),
        locked: false,
        prunable: false,
    };

    let cases = [
        (
            super::super::IssueMonitorTrustCandidate {
                issue_number: 42,
                project_root: other_project.path().to_path_buf(),
                session_mode: gwt_agent::SessionMode::Normal,
            },
            base_config(),
            valid_entry.clone(),
            "project root",
        ),
        (
            valid_candidate.clone(),
            gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
                .working_dir(worktree.path())
                .branch("work/issue-42")
                .linked_issue_number(43)
                .build(),
            valid_entry.clone(),
            "linked Issue",
        ),
        (
            super::super::IssueMonitorTrustCandidate {
                session_mode: gwt_agent::SessionMode::Resume,
                ..valid_candidate.clone()
            },
            base_config(),
            valid_entry.clone(),
            "session mode",
        ),
        (
            valid_candidate.clone(),
            gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
                .working_dir(worktree.path())
                .branch("feature/arbitrary")
                .linked_issue_number(42)
                .build(),
            valid_entry.clone(),
            "branch",
        ),
        (
            valid_candidate,
            base_config(),
            gwt_git::WorktreeInfo {
                prunable: true,
                ..valid_entry
            },
            "authoritative gwt worktree",
        ),
    ];

    for (candidate, config, entry, expected) in cases {
        let error = super::super::validate_issue_monitor_managed_codex_worktree(
            Some(&candidate),
            project.path(),
            &config,
            &[entry],
        )
        .expect_err("mismatched provenance must fail closed");
        assert!(
            error.contains(expected),
            "expected {expected:?} in {error:?}"
        );
    }
}

#[test]
fn codex_hook_trust_launch_uses_effective_codex_home_config() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");
    let codex_home = tempdir().expect("codex home");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();
    let mut launch_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.path())
        .build();
    launch_config.env_vars.insert(
        "CODEX_HOME".to_string(),
        codex_home.path().display().to_string(),
    );

    let report = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &launch_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        None,
    )
    .unwrap()
    .expect("Codex launch should register trust into the effective CODEX_HOME");

    let codex_home_config = gwt_core::paths::normalize_windows_child_process_path(
        &fs::canonicalize(codex_home.path()).unwrap(),
    )
    .join("config.toml");
    assert_eq!(report.config_path, codex_home_config);
    let config = fs::read_to_string(&codex_home_config).unwrap();
    assert!(
        config.contains("trusted_hash"),
        "effective CODEX_HOME config should contain trusted hashes, got: {config}"
    );
    assert!(
        !home.path().join(".codex/config.toml").exists(),
        "backend override launches must not write trust state to the profile-derived Codex home"
    );
}

#[test]
fn codex_hook_trust_launch_defaults_to_host_codex_registration_and_false_opts_out() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let codex_home = home.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex home");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();
    let mut codex_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.path())
        .build();
    codex_config
        .env_vars
        .insert("CODEX_HOME".to_string(), codex_home.display().to_string());

    let unset = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &codex_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        None,
    )
    .unwrap();
    assert_eq!(
        unset
            .expect("unset config should default to trusting managed Codex hooks")
            .trusted_entries
            .len(),
        5
    );
    fs::remove_file(home.path().join(".codex/config.toml")).unwrap();

    let mut settings = Settings::default();
    settings.agent.codex_trust_managed_hooks = Some(false);
    settings.save(&profile_config_path).unwrap();
    let disabled = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &codex_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        None,
    )
    .unwrap();
    assert!(disabled.is_none());

    assert!(
        !home.path().join(".codex/config.toml").exists(),
        "false opt-out must not recreate Codex config"
    );

    settings.agent.codex_trust_managed_hooks = Some(true);
    settings.save(&profile_config_path).unwrap();
    let enabled = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &codex_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        None,
    )
    .unwrap();
    assert_eq!(
        enabled
            .expect("true config should register managed Codex hooks")
            .trusted_entries
            .len(),
        5
    );

    let mut claude_config = codex_config.clone();
    claude_config.agent_id = gwt_agent::AgentId::ClaudeCode;
    let claude = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &claude_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        None,
    )
    .unwrap();
    assert!(claude.is_none());

    assert!(
        home.path().join(".codex/config.toml").exists(),
        "host default/true paths must create Codex config"
    );
}

/// Issue #3967 AC-3: a Monitor launch into a fresh linked worktree must leave
/// no `.codex/hooks.json` that Codex would flag as "new or changed" — neither
/// the worktree-local copy nor the workspace-home copy.
#[test]
fn codex_hook_trust_launch_trusts_every_discovered_worktree_hook_file() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let codex_home = home.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex home");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let fixture_root = tempdir().expect("fixture tempdir");
    let (repo, worktree) = codex_hook_trust_linked_worktree_fixture(fixture_root.path());

    // A worktree created by an older launch keeps its portable local command
    // while the current launch targets workspace-home discovery.
    {
        let _old_bin =
            gwt_skills::settings_local::ScopedHookBin::set(gwt_skills::CANONICAL_HOOK_BIN);
        gwt_skills::generate_codex_hooks_for_mode(
            &worktree,
            gwt_skills::CodexHookDiscoveryMode::WorktreeLocal,
        )
        .expect("seed old local hooks");
    }
    let materialization = gwt::refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
        &worktree,
        &gwt_agent::AgentId::Codex,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        false,
    )
    .expect("refresh launch assets");

    let mut launch_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&worktree)
        .build();
    launch_config
        .env_vars
        .insert("CODEX_HOME".to_string(), codex_home.display().to_string());

    let report = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        &worktree,
        &launch_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        materialization.hook_bin.as_deref(),
    )
    .expect("launch trust registration must succeed")
    .expect("Codex host launch registers trust");

    assert!(
        report.untrusted_gwt_hooks.is_empty(),
        "no gwt hook may be left for human review: {report:?}"
    );
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(&report.config_path).expect("read codex config"))
            .expect("parse codex config");
    // Issue #4879: take both paths from the product's own discovery derivation.
    // Joining `.codex/hooks.json` onto the fixture roots by hand produced a
    // workspace-home path that only looked right: the registration side reaches
    // that copy through git's `gitdir`, so the hand-built form differed from the
    // key it had written, and the assertion failed on a correct product.
    let worktree_local = gwt_skills::codex_hooks_paths_for_codex_discovery(
        &worktree,
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal,
    );
    let workspace_home = gwt_skills::codex_hooks_paths_for_codex_discovery(
        &worktree,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    );
    assert_ne!(
        worktree_local, workspace_home,
        "fixture must be a linked worktree whose two hook copies are distinct"
    );
    assert!(
        workspace_home
            .iter()
            .all(|path| path.starts_with(dunce::canonicalize(&repo).expect("canonical repo"))),
        "the workspace-home copy must live under the repository checkout: \
         {workspace_home:?} vs {repo:?}"
    );
    for path in worktree_local.iter().chain(workspace_home.iter()) {
        assert_every_codex_hook_is_trusted(&config, path);
    }
}

/// Issue #3967 AC-4: a pre-registration that cannot vouch for the gwt hooks
/// must fail the launch with a concrete reason instead of handing the agent a
/// human-only prompt that silently holds the Issue Monitor slot.
#[test]
fn codex_hook_trust_launch_fails_when_a_gwt_hook_cannot_be_trusted() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let codex_home = home.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex home");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();
    let hooks_path = worktree.path().join(".codex/hooks.json");
    let mut hooks_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&hooks_path).unwrap()).unwrap();
    hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] =
        serde_json::Value::String("'/tmp/attacker/gwtd' hook event Stop".to_string());
    fs::write(
        &hooks_path,
        serde_json::to_string_pretty(&hooks_json).unwrap(),
    )
    .unwrap();

    let mut launch_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.path())
        .build();
    launch_config
        .env_vars
        .insert("CODEX_HOME".to_string(), codex_home.display().to_string());

    let result = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &launch_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        None,
    );

    let error = result.expect_err("untrusted gwt hook must abort the launch");
    assert!(
        error.contains("Hooks need review") && error.contains("stop:0:0"),
        "launch failure must name the cause and the hook: {error}"
    );
}

/// Issue #3967 AC-4: an unwritable Codex config used to be swallowed as a
/// warning, which launched the agent straight into `Hooks need review` and let
/// the pane hold its Issue Monitor slot in silence. The launch now fails with
/// the reason so the slot is released and the cause is visible.
#[test]
fn codex_hook_trust_launch_fails_when_codex_config_cannot_be_written() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();

    let codex_config_parent = home.path().join(".codex");
    fs::write(&codex_config_parent, "not a directory").unwrap();
    let mut launch_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.path())
        .build();
    launch_config.env_vars.insert(
        "CODEX_HOME".to_string(),
        codex_config_parent.display().to_string(),
    );

    let result = super::super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &launch_config,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        None,
    );

    let error = result.expect_err("unwritable Codex trust state must abort the launch");
    assert!(
        error.contains("Codex hook trust"),
        "launch failure must name the cause: {error}"
    );
}

#[test]
fn workspace_view_for_tab_omits_work_item_history_from_workspace_state() {
    // SPEC-2359 CPU/power follow-up: workspace_state is broadcast frequently
    // and must stay structural. Workspace history/work items are carried by
    // active_work_projection so every window/status update does not serialize
    // the full work item event log.
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");

    use chrono::TimeZone as _;
    let completed_at = chrono::Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap();
    let work_item = gwt_core::workspace_projection::WorkItem {
        id: "work-item-done".to_string(),
        title: "Test Done Item".to_string(),
        intent: None,
        summary: None,
        progress_summary: None,
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
        owner: None,
        created_at: completed_at,
        updated_at: completed_at,
        completed_at: Some(completed_at),
        agents: Vec::new(),
        execution_containers: Vec::new(),
        board_refs: Vec::new(),
        related_work_item_ids: Vec::new(),
        events: Vec::new(),
        legacy_metadata_snapshot: None,
        legacy_metadata_authoritative: false,
        legacy_metadata_snapshot_at: None,
        duplicate_event_containers: Default::default(),
        discarded: false,
        discarded_at: None,
    };
    let projection = gwt_core::workspace_projection::WorkItemsProjection {
        updated_at: completed_at,
        work_items: vec![work_item],
    };
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    fs::create_dir_all(work_items_path.parent().expect("parent dir"))
        .expect("create workspace dir");
    gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
        &work_items_path,
        &projection,
    )
    .expect("save work items projection");

    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(empty_workspace_state()),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };

    let view = crate::runtime_support::workspace_view_for_tab(&tab);
    assert!(
            view.work_items.is_empty(),
            "WorkspaceView.work_items must stay empty because workspace_state is a hot broadcast path; active_work_projection owns Workspace history"
        );
}

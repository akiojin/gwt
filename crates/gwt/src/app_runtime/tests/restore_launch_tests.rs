use super::*;

/// Read the same immutable source metadata and canonical restore predicates as
/// the owner. These history/refusal cases do not claim that a PTY was started;
/// the small owner restore cases below verify physical publication and removal.
struct RestoreFixtureAdmission {
    admitted: Vec<String>,
    terminal: HashSet<String>,
    log: super::super::startup::RestoreAdmissionLog,
}

fn assess_restore_fixture(runtime: &AppRuntime, tab_id: &str) -> RestoreFixtureAdmission {
    use super::super::startup::{RestoreAdmissionLog, RestoreRefusal};
    let tab = runtime.tab(tab_id).expect("restore fixture tab");
    let mut result = RestoreFixtureAdmission {
        admitted: Vec::new(),
        terminal: HashSet::new(),
        log: RestoreAdmissionLog::default(),
    };
    for window in &tab.workspace.persisted().windows {
        let Some(id) = window.session_id.as_ref() else {
            continue;
        };
        let source = gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{id}.toml")))
            .expect("saved restore admission source");
        match runtime.restore_admission(
            &source,
            &tab.project_root,
            Some(&combined_window_id(tab_id, &window.id)),
        ) {
            Ok(()) => result.admitted.push(source.id),
            Err(refusal) => {
                if matches!(refusal, RestoreRefusal::TerminalWork(_)) {
                    result.terminal.insert(source.id.clone());
                }
                result.log.refuse(&source.id, refusal);
            }
        }
    }
    result.log.emit_summary("domain_admission", 0);
    result
}

#[test]
fn issue_restart_preserves_its_member_when_every_initial_is_reserved() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut settings = Settings::default();
    settings.ai.language = Some("en".into());
    settings
        .save(&Settings::global_config_path().unwrap())
        .unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let placeholders: Vec<_> = ('A'..='Z')
        .map(|initial| (format!("agent-{initial}"), format!("session-{initial}")))
        .collect();
    let mut tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    for initial in 'A'..='Z' {
        tab.workspace.set_agent_name(
            &format!("agent-{initial}"),
            Some(format!("{initial}Legacy")),
        );
    }
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = gwt_agent::Session::new(&repo, "work/restart", gwt_agent::AgentId::Codex);
    session.id = "session-F".into();
    session.agent_session_id = Some("native-restart".into());
    session.agent_name = Some("FLegacy".into());
    session.linked_issue_number = Some(3824);
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.save(&runtime.sessions_dir).unwrap();

    runtime.spawn_restored_agent_session(
        "tab-1",
        session,
        None,
        super::super::startup::RestoreOrigin::UserRequested,
    );

    assert_eq!(
        runtime.pending_auto_resume_sources.len(),
        1,
        "restart admitted"
    );
    let windows = &runtime.tabs[0].workspace.persisted().windows;
    assert_eq!(windows.len(), 26);
    let replacement = windows
        .iter()
        .filter_map(|window| window.agent_name.as_deref())
        .find(|name| name.starts_with('F'))
        .expect("the same member keeps its saved name");
    assert_eq!(replacement, "FLegacy");
    assert!(!replacement.chars().any(|ch| ch.is_ascii_digit()));
}

#[test]
fn automatic_restore_preserves_names_even_when_saved_initials_collide() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    let mut tab = restore_fixture_tab(
        "tab-1",
        &repo,
        &[
            ("agent-1".into(), "session-1".into()),
            ("agent-2".into(), "session-2".into()),
        ],
    );
    for (id, name) in [("agent-1", "モナ"), ("agent-2", "モミジ")] {
        tab.workspace.set_agent_name(id, Some(name.into()));
    }
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let (command, args) = long_running_test_command();
    persist_owner_fixture_agent(command, args);
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let _owner = project_owner_fixture(&mut runtime, "tab-1::agent-1");
    for (id, name) in [("session-1", "モナ"), ("session-2", "モミジ")] {
        let worktree = temp.path().join(id);
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                &format!("work/{id}"),
                worktree.to_str().unwrap(),
            ],
        );
        save_restore_fixture_session(
            &runtime.sessions_dir,
            id,
            &temp.path().join(id),
            Some(id),
            None,
        );
        let path = runtime.sessions_dir.join(format!("{id}.toml"));
        let mut session = gwt_agent::Session::load(&path).unwrap();
        session.agent_name = Some(name.into());
        session.agent_id = gwt_agent::AgentId::Custom("fixture".into());
        session.branch = format!("work/{id}");
        session.project_state_root = Some(repo.clone());
        session.save(&runtime.sessions_dir).unwrap();
    }

    runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    let events = runtime.startup_auto_resume_ready_events();

    assert_eq!(runtime.pending_auto_resume_sources.len(), 2, "{events:?}");
    assert_eq!(runtime.restore_launch_windows.len(), 2);
    let windows = &runtime.tabs[0].workspace.persisted().windows;
    assert_eq!(windows.len(), 2);
    let names: Vec<_> = windows
        .iter()
        .map(|window| window.agent_name.as_deref().expect("restored name"))
        .collect();
    assert_ne!(names[0], names[1]);
    assert!(names.contains(&"モナ") && names.contains(&"モミジ"));
    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::LaunchWizardOpenError { .. })));
    let actual_events = drain_owner_launch_fixtures(&mut runtime, &tasks);
    assert_eq!(
        runtime.runtimes.len(),
        2,
        "actual restore receipts: {actual_events:?}"
    );
    assert!(runtime.restore_launch_windows.is_empty());
    let roster = gwt::agent_names::read_project_roster(&repo, &runtime.sessions_dir).unwrap();
    assert_eq!(
        roster.enabled_count(),
        1,
        "restore does not enlarge the configured capacity"
    );
    assert_eq!(
        roster.members.len(),
        2,
        "the excess restored member is retained"
    );
    for window_id in runtime.runtimes.keys().cloned().collect::<Vec<_>>() {
        close_test_owner_runtime(&mut runtime, &window_id);
    }
}

#[test]
fn restore_name_allocation_failure_notifies_automatically_and_keeps_manual_errors() {
    use super::super::startup::RestoreOrigin;

    for origin in [RestoreOrigin::Automatic, RestoreOrigin::UserRequested] {
        for placeholder in [false, true] {
            let temp = tempdir().expect("tempdir");
            let _gwt_home = ScopedGwtHome::set(temp.path());
            let repo = temp.path().join("repo");
            fs::create_dir_all(&repo).unwrap();
            init_repo_with_initial_commit(&repo);
            let mut placeholders = vec![("occupied".into(), "other-session".into())];
            if placeholder {
                placeholders.push(("restoring".into(), "restore-session".into()));
            }
            let mut tab = restore_fixture_tab("tab-1", &repo, &placeholders);
            tab.workspace
                .set_agent_name("occupied", Some("モミジ".into()));
            if placeholder {
                tab.workspace
                    .set_agent_name("restoring", Some("モナ".into()));
            }
            let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
            runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
            let (spawner, tasks) = BlockingTaskSpawner::queued();
            runtime.blocking_tasks = spawner;
            let executor = tokio::runtime::Runtime::new().unwrap();
            let owner = executor.block_on(production_owner_fixture(&repo));
            let worktree = temp.path().join("restoring-worktree");
            fs::create_dir_all(&worktree).unwrap();
            let mut session = gwt_agent::Session::new(
                &worktree,
                "work/restore-fixture",
                gwt_agent::AgentId::Codex,
            );
            session.id = "restore-session".into();
            session.agent_session_id = Some("native-restore".into());
            session.agent_name = Some("モナ".into());
            session.linked_issue_number = Some(3824);
            session.project_state_root = Some(repo.clone());
            session.record_hook_event("Stop");
            session.record_completed_stop();
            session.save(&runtime.sessions_dir).unwrap();
            // Make the existing allocator fail after detecting the collision.
            fs::create_dir_all(
                gwt_core::paths::gwt_project_dir_for_repo_path(&repo).join("agent-names.jsonl"),
            )
            .unwrap();

            let mut events = runtime.spawn_restored_agent_session("tab-1", session, None, origin);
            events.extend(drain_owner_launch_fixtures(&mut runtime, &tasks));
            executor.block_on(owner.shutdown());
            assert!(runtime.pending_auto_resume_sources.is_empty());
            assert_eq!(
                events.iter().any(|event| matches!(event.event,
                    BackendEvent::IssueMonitorToast { ref level, .. } if level == "error")),
                origin == RestoreOrigin::Automatic,
                "automatic failures need a notification even when a placeholder exists"
            );
            assert_eq!(
                events
                    .iter()
                    .any(|event| matches!(event.event, BackendEvent::LaunchWizardOpenError { .. })),
                origin == RestoreOrigin::UserRequested && !placeholder
            );
            if placeholder {
                assert_eq!(
                    runtime.window_status(&combined_window_id("tab-1", "restoring")),
                    Some(WindowProcessStatus::Error)
                );
            }
        }
    }
}

#[test]
fn restore_admits_worktree_with_unlanded_commits() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/develop", "HEAD"],
    );
    run_git(&repo, &["commit", "--allow-empty", "-m", "unlanded work"]);
    let runtime = sample_runtime(temp.path(), vec![], None);
    let mut session = gwt_agent::Session::new(&repo, "work/live", gwt_agent::AgentId::Codex);
    session.agent_session_id = Some("native-live".into());
    let git_spawns = gwt_core::process::thread_git_spawn_count();
    assert_eq!(runtime.restore_admission(&session, &repo, None), Ok(()));
    assert_eq!(
        gwt_core::process::thread_git_spawn_count() - git_spawns,
        1,
        "the merge-base restore check must participate in startup Git measurements"
    );
}

#[test]
fn restore_closed_diagnostic_keeps_placeholder_without_spawning() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let tab = restore_fixture_tab(
        "tab-closed",
        &repo,
        &[("agent-closed".into(), "session-closed".into())],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-closed"));
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-closed",
        &repo,
        Some("native-closed"),
        Some(4143),
    );
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            merged_issues: vec![4143],
            ..Default::default()
        },
    )
    .expect("merged owner");
    let path = runtime.sessions_dir.join("session-closed.toml");
    let mut session = gwt_agent::Session::load(&path).unwrap();
    session.status = gwt_agent::AgentStatus::Interrupted;
    session.save(&runtime.sessions_dir).unwrap();
    assert_eq!(
        runtime.restore_admission(&session, &repo, Some("tab-closed::agent-closed")),
        Err(super::super::startup::RestoreRefusal::ClosedWorkDiagnostic)
    );
    let admission = assess_restore_fixture(&runtime, "tab-closed");
    assert!(admission.admitted.is_empty());
    assert!(admission.terminal.is_empty());
    assert_eq!(runtime.tabs[0].workspace.persisted().windows.len(), 1);
}

#[test]
fn restore_pending_worktree_reservation_ends_when_window_closes() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let tab = restore_fixture_tab(
        "tab-pending",
        &repo,
        &[("agent-pending".into(), "session-pending".into())],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-pending"));
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-pending",
        &repo,
        Some("native-pending"),
        None,
    );
    let session =
        gwt_agent::Session::load(&runtime.sessions_dir.join("session-pending.toml")).unwrap();
    let window = combined_window_id("tab-pending", "agent-pending");
    runtime
        .pending_auto_resume_sources
        .insert(window.clone(), session.id.clone());
    assert_eq!(
        runtime.restore_admission(&session, &repo, None),
        Err(super::super::startup::RestoreRefusal::WorktreeAlreadyRestoring)
    );
    runtime.close_window_after_issue_monitor_finalize_events(&window);
    assert_eq!(runtime.restore_admission(&session, &repo, None), Ok(()));
}

#[test]
fn restore_summary_waits_for_async_preparation_failure_in_each_restore_route() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    for startup in [true, false] {
        let root = temp
            .path()
            .join(if startup { "startup" } else { "open-project" });
        let repo = root.join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let placeholders = vec![("agent-restore".to_string(), "session-restore".to_string())];
        let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
        let mut runtime = sample_runtime(&root, vec![tab], Some("tab-1"));
        runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        let _owner = project_owner_fixture(&mut runtime, "tab-1::agent-restore");
        save_restore_fixture_session(
            &runtime.sessions_dir,
            "session-restore",
            &repo,
            Some("native"),
            None,
        );
        let path = runtime.sessions_dir.join("session-restore.toml");
        let mut source = gwt_agent::Session::load(&path).expect("source Session");
        source.agent_id = gwt_agent::AgentId::Custom("missing-restore-provider".into());
        source.project_state_root = Some(repo.clone());
        source
            .save(&runtime.sessions_dir)
            .expect("actual preflight failure source");
        let prepared = capture_tracing_events(|| {
            if startup {
                runtime.queue_startup_auto_resume_sessions(&HashSet::new());
                runtime.startup_auto_resume_ready_events();
            } else {
                runtime.restore_open_project_windows("tab-1");
            }
        });
        assert!(
            prepared
                .iter()
                .all(|event| event.fields.get("message").map(String::as_str)
                    != Some("session restore admission summary")),
            "preparation is not a completed restore"
        );
        let window_id = runtime
            .restore_launch_windows
            .keys()
            .next()
            .expect("pending restore")
            .clone();
        let completed = capture_tracing_events(|| {
            drain_owner_launch_fixtures(&mut runtime, &tasks);
        });
        let summary = restore_admission_summary(&completed);
        assert_eq!(
            summary.fields.get("restored").map(String::as_str),
            Some("0")
        );
        assert_eq!(summary.fields.get("skipped").map(String::as_str), Some("1"));
        assert_eq!(
            summary.fields.get("reasons").map(String::as_str),
            Some("launch_not_started=1")
        );
        assert!(!runtime.window_lookup.contains_key(&window_id));
    }
}

#[test]
fn startup_restore_summary_includes_missing_worktrees_before_queue_and_before_spawn() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let placeholders = vec![
        ("agent-missing".to_string(), "session-missing".to_string()),
        ("agent-removed".to_string(), "session-removed".to_string()),
    ];
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for session_id in ["session-missing", "session-removed"] {
        save_restore_fixture_session(
            &runtime.sessions_dir,
            session_id,
            &temp.path().join(session_id),
            Some(session_id),
            None,
        );
    }
    fs::remove_dir(temp.path().join("session-missing")).expect("remove stale worktree");
    let logs = capture_tracing_events(|| {
        runtime.queue_startup_auto_resume_sessions(&HashSet::new());
        assert_eq!(runtime.pending_startup_auto_resume_sessions.len(), 1);
        fs::remove_dir(temp.path().join("session-removed")).expect("remove queued worktree");
        runtime.startup_auto_resume_ready_events();
        runtime.startup_auto_resume_ready_events();
    });
    assert!(runtime.pending_auto_resume_sources.is_empty());
    let summary = restore_admission_summary(&logs);
    assert_eq!(
        summary.fields.get("restored").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        summary.fields.get("suppressed").map(String::as_str),
        Some("2")
    );
    assert_eq!(
        summary.fields.get("reasons").map(String::as_str),
        Some("worktree_missing=2")
    );
}

#[test]
fn open_project_restore_reports_a_missing_worktree_in_its_summary() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let placeholders = vec![("agent-missing".to_string(), "session-missing".to_string())];
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let worktree = temp.path().join("missing");
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-missing",
        &worktree,
        Some("native"),
        None,
    );
    fs::remove_dir(&worktree).expect("remove worktree");
    let logs = capture_tracing_events(|| {
        runtime.restore_open_project_windows("tab-1");
    });
    assert!(runtime.pending_auto_resume_sources.is_empty());
    let summary = restore_admission_summary(&logs);
    assert_eq!(
        summary.fields.get("restored").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        summary.fields.get("reasons").map(String::as_str),
        Some("worktree_missing=1")
    );
}

#[test]
fn restored_session_rechecks_that_its_worktree_is_a_directory_before_spawning() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = restore_fixture_tab("tab-1", &repo, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let worktree = temp.path().join("not-a-directory");
    fs::write(&worktree, "replaced worktree").expect("write file");
    let mut source = gwt_agent::Session::new(&worktree, "main", gwt_agent::AgentId::Codex);
    source.agent_session_id = Some("native".to_string());
    let events = runtime.spawn_restored_agent_session(
        "tab-1",
        source,
        None,
        super::super::startup::RestoreOrigin::Automatic,
    );
    assert!(events.is_empty());
    assert!(runtime.pending_auto_resume_sources.is_empty());
    assert!(runtime.tabs[0].workspace.persisted().windows.is_empty());
}

/// Issue #4143 AC-2 / AC-4: restore admits only a window that both has a
/// resumable agent session and a Work that is not terminal, records the reason
/// for every refusal, and summarises the sweep in one line.
#[test]
fn startup_restore_admits_only_resumable_sessions_with_live_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let placeholders = vec![
        ("agent-live".to_string(), "session-live".to_string()),
        (
            "agent-no-resume".to_string(),
            "session-no-resume".to_string(),
        ),
        ("agent-closed".to_string(), "session-closed".to_string()),
    ];
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let (command, args) = long_running_test_command();
    persist_owner_fixture_agent(command, args);
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    for (session_id, suffix) in [
        ("session-live", "live"),
        ("session-no-resume", "no-resume"),
        ("session-closed", "closed"),
    ] {
        let worktree = temp.path().join(format!("wt-{suffix}"));
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                &format!("work/{session_id}"),
                worktree.to_str().unwrap(),
            ],
        );
    }
    let _owner = project_owner_fixture(&mut runtime, "tab-1::agent-live");
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-live",
        &temp.path().join("wt-live"),
        Some("native-live"),
        Some(4001),
    );
    // Predicate 1: no conversation handle, so a restore could only ever
    // produce an idle pane.
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-no-resume",
        &temp.path().join("wt-no-resume"),
        None,
        Some(4002),
    );
    // Predicate 2: the linked Work is durably complete.
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-closed",
        &temp.path().join("wt-closed"),
        Some("native-closed"),
        Some(4003),
    );
    for session_id in ["session-live", "session-no-resume", "session-closed"] {
        let path = runtime.sessions_dir.join(format!("{session_id}.toml"));
        let mut session = gwt_agent::Session::load(&path).expect("source Session");
        session.agent_id = gwt_agent::AgentId::Custom("fixture".into());
        session.branch = format!("work/{session_id}");
        session.project_state_root = Some(repo.clone());
        session
            .save(&runtime.sessions_dir)
            .expect("fixed project source");
    }
    let prefs = gwt::IssueMonitorPrefs {
        merged_issues: vec![4003],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("seed monitor prefs");

    let logs = capture_tracing_events(|| {
        runtime.queue_startup_auto_resume_sessions(&HashSet::new());
        assert_eq!(
            runtime.pending_startup_auto_resume_sessions.len(),
            3,
            "canonical Work/native admission is deferred to the actual owner"
        );
        runtime.startup_auto_resume_ready_events();
        drain_owner_launch_fixtures(&mut runtime, &tasks);
    });

    let refusals = restore_admission_refusals(&logs);
    assert_eq!(
        refusals.get("session-no-resume").map(String::as_str),
        Some("no_resume_session"),
        "AC-2 requires the per-target refusal reason in the log: {refusals:?}"
    );
    assert_eq!(
        refusals.get("session-closed").map(String::as_str),
        Some("terminal_work:closed_issue"),
        "AC-2 requires the per-target refusal reason in the log: {refusals:?}"
    );

    let summary = restore_admission_summary(&logs);
    assert_eq!(
        summary.fields.get("restored").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        summary.fields.get("suppressed").map(String::as_str),
        Some("2")
    );
    let reasons = summary.fields.get("reasons").cloned().unwrap_or_default();
    assert!(
        reasons.contains("no_resume_session=1") && reasons.contains("terminal_work:closed_issue=1"),
        "AC-4 requires the reason breakdown in the summary line, got {reasons:?}"
    );

    // A terminal window stops coming back: its placeholder is gone and the
    // Session is restore-disabled.
    let closed = gwt_agent::Session::load(&runtime.sessions_dir.join("session-closed.toml"))
        .expect("load terminal session");
    assert!(!closed.restore_window_on_startup);
    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .all(|window| window.session_id.as_deref() != Some("session-closed")));
    assert_eq!(
        runtime.runtimes.len(),
        1,
        "only the actually admitted live Work has a PTY"
    );
    for window_id in runtime.runtimes.keys().cloned().collect::<Vec<_>>() {
        close_test_owner_runtime(&mut runtime, &window_id);
    }
}

/// Issue #4143 AC-2: an unreadable Work fact is not evidence that the window is
/// finished, and it is not permission to respawn either. The observed incident
/// began exactly here — descriptor exhaustion (#4142) made the Monitor prefs
/// unreadable, and every one of 254 windows respawned on that unreadable fact.
#[test]
fn startup_restore_refuses_and_keeps_the_placeholder_when_work_facts_are_unreadable() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let placeholders = vec![("agent-1".to_string(), "session-unreadable".to_string())];
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-unreadable",
        &temp.path().join("wt-unreadable"),
        Some("native-unreadable"),
        Some(4143),
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    fs::create_dir_all(prefs_path.parent().expect("prefs parent")).expect("create prefs dir");
    fs::write(&prefs_path, b"{ this is not monitor prefs").expect("write unreadable prefs");

    let mut admission = None;
    let logs = capture_tracing_events(|| {
        admission = Some(assess_restore_fixture(&runtime, "tab-1"));
    });
    let admission = admission.unwrap();

    assert!(
        admission.admitted.is_empty(),
        "an unprovable Work must not respawn"
    );
    let refusals = restore_admission_refusals(&logs);
    assert_eq!(
        refusals.get("session-unreadable").map(String::as_str),
        Some("terminal_facts_unreadable:monitor_unreadable"),
        "{refusals:?}"
    );
    // The placeholder and the restore flag survive: the next generation may
    // be able to read the answer.
    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .any(|window| window.session_id.as_deref() == Some("session-unreadable")));
    let session = gwt_agent::Session::load(&runtime.sessions_dir.join("session-unreadable.toml"))
        .expect("load session");
    assert!(
        session.restore_window_on_startup,
        "an unreadable fact must not disable restore permanently"
    );
}

/// Issue #4143 AC-5: a large history collapses to the windows whose Work is
/// still live.
#[test]
fn startup_restore_limits_a_large_history_to_unterminated_windows() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    const TERMINAL: usize = 160;
    const LIVE: usize = 50;

    let mut placeholders = Vec::new();
    let mut merged_issues = Vec::new();
    for index in 0..TERMINAL {
        merged_issues.push(500_000 + index as u64);
        placeholders.push((
            format!("agent-done-{index}"),
            format!("session-done-{index}"),
        ));
    }
    for index in 0..LIVE {
        placeholders.push((
            format!("agent-live-{index}"),
            format!("session-live-{index}"),
        ));
    }
    assert!(placeholders.len() >= 200);
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for index in 0..TERMINAL {
        save_restore_fixture_session(
            &runtime.sessions_dir,
            &format!("session-done-{index}"),
            &temp.path().join(format!("wt-done-{index}")),
            Some(&format!("native-done-{index}")),
            Some(500_000 + index as u64),
        );
    }
    for index in 0..LIVE {
        save_restore_fixture_session(
            &runtime.sessions_dir,
            &format!("session-live-{index}"),
            &temp.path().join(format!("wt-live-{index}")),
            Some(&format!("native-live-{index}")),
            Some(600_000 + index as u64),
        );
    }
    let prefs = gwt::IssueMonitorPrefs {
        merged_issues,
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("seed monitor prefs");

    let admission = assess_restore_fixture(&runtime, "tab-1");
    assert_eq!(
        admission.admitted.len(),
        LIVE,
        "only the windows whose Work is still live may restore"
    );
    assert!(admission
        .admitted
        .iter()
        .all(|id| id.starts_with("session-live-")));
    assert_eq!(
        admission.log.reasons(),
        format!("terminal_work:closed_issue={TERMINAL}")
    );
    assert_eq!(admission.terminal.len(), TERMINAL);
}

#[test]
fn startup_restore_limits_windows_to_the_last_open_set() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    const HISTORY: usize = 10;

    let mut placeholders = vec![
        ("agent-open-0".to_string(), "session-open-0".to_string()),
        ("agent-open-1".to_string(), "session-open-1".to_string()),
    ];
    for index in 0..HISTORY {
        placeholders.push((
            format!("agent-history-{index}"),
            format!("session-history-{index}"),
        ));
    }
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    for index in 0..2 {
        save_restore_fixture_session(
            &runtime.sessions_dir,
            &format!("session-open-{index}"),
            &temp.path().join(format!("wt-open-{index}")),
            Some(&format!("native-open-{index}")),
            Some(4_441_000 + index as u64),
        );
    }
    for index in 0..HISTORY {
        let session_id = format!("session-history-{index}");
        save_restore_fixture_session(
            &runtime.sessions_dir,
            &session_id,
            &temp.path().join(format!("wt-history-{index}")),
            Some(&format!("native-history-{index}")),
            Some(4_442_000 + index as u64),
        );
        // Every one of these kept its placeholder: nobody closed the window by
        // hand, which is exactly why they accumulated.
        age_restore_fixture_session(
            &runtime.sessions_dir,
            &session_id,
            chrono::Duration::hours(48),
        );
    }

    let mut queued = Vec::new();
    let logs = capture_tracing_events(|| {
        runtime.queue_startup_auto_resume_sessions(&HashSet::new());
        queued = runtime
            .pending_startup_auto_resume_sessions
            .iter()
            .map(|pending| pending.session.id.clone())
            .collect();
        // Issue #4305 (AC-5): the summary counts restores that actually
        // reached a PTY, so it is emitted from the canvas-ready drain once
        // every prepared window has settled — not from the selection pass.
        runtime.startup_auto_resume_ready_events();
        let prepared = runtime
            .restore_launch_windows
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for window_id in prepared {
            runtime.record_restore_window_outcome(&window_id, Ok(()));
        }
    });

    let mut restored = queued;
    restored.sort();
    assert_eq!(
        restored,
        vec!["session-open-0".to_string(), "session-open-1".to_string()],
        "AC-1 requires only the last-open set to restore"
    );

    let refusals = restore_admission_refusals(&logs);
    for index in 0..HISTORY {
        assert_eq!(
            refusals
                .get(&format!("session-history-{index}"))
                .map(String::as_str),
            Some("stale"),
            "AC-1: a placeholder must not exempt history from the freshness bound: {refusals:?}"
        );
    }

    // AC-5: the restored count and the time the sweep took are both on the
    // one summary line, so "restore is slow" is measurable next time.
    let summary = restore_admission_summary(&logs);
    assert_eq!(
        summary.fields.get("restored").map(String::as_str),
        Some("2")
    );
    assert_eq!(
        summary.fields.get("suppressed").map(String::as_str),
        Some(HISTORY.to_string().as_str())
    );
    assert!(
        summary.fields.contains_key("elapsed_ms"),
        "AC-5 requires the selection duration on the summary line, got {:?}",
        summary.fields
    );
}

/// Issue #4441 AC-2: one owner Issue restores one window.
///
/// Each relaunch mints a fresh conversation handle, so the existing
/// native-session dedupe never collapsed them — the reporting fleet restored
/// `#4257` eight times.
#[test]
fn startup_restore_collapses_duplicate_owner_issue_windows() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");

    let placeholders = (0..3)
        .map(|index| {
            (
                format!("agent-relaunch-{index}"),
                format!("session-relaunch-{index}"),
            )
        })
        .collect::<Vec<_>>();
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for index in 0..3 {
        let session_id = format!("session-relaunch-{index}");
        save_restore_fixture_session(
            &runtime.sessions_dir,
            &session_id,
            &temp.path().join(format!("wt-relaunch-{index}")),
            Some(&format!("native-relaunch-{index}")),
            Some(4257),
        );
        // The newest relaunch is the one that should come back.
        age_restore_fixture_session(
            &runtime.sessions_dir,
            &session_id,
            chrono::Duration::minutes(10 * (2 - index as i64)),
        );
    }

    let logs = capture_tracing_events(|| {
        runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    });

    let restored = runtime
        .pending_startup_auto_resume_sessions
        .iter()
        .map(|pending| pending.session.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        restored,
        vec!["session-relaunch-2"],
        "AC-2 requires one window per owner Issue, newest first"
    );
    let refusals = restore_admission_refusals(&logs);
    for index in 0..2 {
        assert_eq!(
            refusals
                .get(&format!("session-relaunch-{index}"))
                .map(String::as_str),
            Some("duplicate_owner_issue"),
            "{refusals:?}"
        );
    }
}

/// Issue #4441 AC-3: a row the operator stopped with `issue.monitor.stop` does
/// not come back as a restored window on the next startup.
///
/// This is the case that silently defeated the operator's only lever. The stop
/// parks the row for a human and records a failure hold; the close predicate
/// reports both as "do not close"; restore read that as "do spawn". So every
/// row the PM stopped was recreated at the next launch, and the refill looked
/// like volume rather than like the stop itself.
///
/// The fixture drives the Monitor through the same call `stop_only` makes, then
/// persists the prefs, so the test asserts over the durable product of the
/// operator's action rather than over a hand-written flag.
#[test]
fn startup_restore_does_not_resurrect_a_row_stopped_through_the_monitor() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let placeholders = vec![
        ("agent-stopped".to_string(), "session-stopped".to_string()),
        ("agent-live".to_string(), "session-live".to_string()),
    ];
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (session_id, worktree, issue) in [
        ("session-stopped", "wt-stopped", 4286u64),
        ("session-live", "wt-live", 4288),
    ] {
        save_restore_fixture_session(
            &runtime.sessions_dir,
            session_id,
            &temp.path().join(worktree),
            Some(&format!("native-{session_id}")),
            Some(issue),
        );
    }

    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
    // The exact park `IssueMonitorState::stop_only` performs: it is what turns
    // an operator stop into a durable `failed_issues` entry.
    monitor.escalate_to_needs_human(
        4286,
        gwt::NeedsHumanKind::UserChoiceRequired,
        "stopped: PM held this row while adjudicating",
    );
    let prefs = monitor.prefs();
    assert!(
        prefs
            .failed_issues
            .iter()
            .any(|failed| failed.issue_number == 4286),
        "the stop must be durable for this test to mean anything"
    );
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("seed monitor prefs");

    let mut admission = None;
    let logs = capture_tracing_events(|| {
        admission = Some(assess_restore_fixture(&runtime, "tab-1"));
    });
    let admission = admission.unwrap();

    let restored = admission
        .admitted
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        restored,
        vec!["session-live"],
        "a stopped row must not be recreated by the next startup"
    );
    let refusal = restore_admission_refusals(&logs)
        .get("session-stopped")
        .cloned()
        .unwrap_or_default();
    assert!(
        refusal.starts_with("monitor_hold:"),
        "the refusal must name the hold, got {refusal:?}"
    );

    // The stop is reversible: releasing the row must bring the window back, so
    // neither the placeholder nor the restore flag is discarded.
    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .any(|window| window.session_id.as_deref() == Some("session-stopped")));
    let session = gwt_agent::Session::load(&runtime.sessions_dir.join("session-stopped.toml"))
        .expect("load stopped session");
    assert!(session.restore_window_on_startup);
}

/// Issue #4441 AC-3: a window whose Monitor row the operator is holding does
/// not respawn.
///
/// `classify_terminal_window` was written for the *close* side, where an
/// unproven fact must never close a window. Restore reused it and read every
/// such `Ineligible` — `failure_hold` from `issue.monitor.stop` included — as
/// permission to spawn.
#[test]
fn startup_restore_refuses_a_monitor_held_row_and_keeps_the_placeholder() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let placeholders = vec![
        ("agent-held".to_string(), "session-held".to_string()),
        ("agent-parked".to_string(), "session-parked".to_string()),
        ("agent-live".to_string(), "session-live".to_string()),
    ];
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (session_id, worktree, issue) in [
        ("session-held", "wt-held", 4286u64),
        ("session-parked", "wt-parked", 4287),
        ("session-live", "wt-live", 4288),
    ] {
        save_restore_fixture_session(
            &runtime.sessions_dir,
            session_id,
            &temp.path().join(worktree),
            Some(&format!("native-{session_id}")),
            Some(issue),
        );
    }
    let prefs = gwt::IssueMonitorPrefs {
        failed_issues: vec![gwt::IssueMonitorFailedIssue {
            issue_number: 4286,
            message: "operator stop hold".to_string(),
            window_id: None,
        }],
        autonomous_records: vec![gwt::AutonomousIssueRecord {
            phase: gwt::AutonomousPhase::NeedsHuman,
            ..gwt::AutonomousIssueRecord::new(4287)
        }],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("seed monitor prefs");

    let mut admission = None;
    let logs = capture_tracing_events(|| {
        admission = Some(assess_restore_fixture(&runtime, "tab-1"));
    });
    let admission = admission.unwrap();

    let restored = admission
        .admitted
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(restored, vec!["session-live"]);
    let refusals = restore_admission_refusals(&logs);
    assert_eq!(
        refusals.get("session-held").map(String::as_str),
        Some("monitor_hold:failure_hold"),
        "{refusals:?}"
    );
    assert_eq!(
        refusals.get("session-parked").map(String::as_str),
        Some("monitor_hold:needs_human"),
        "{refusals:?}"
    );

    // A hold is not terminal: the window comes back once the operator releases
    // the row, so neither the placeholder nor the restore flag is discarded.
    for session_id in ["session-held", "session-parked"] {
        assert!(
            runtime
                .tab("tab-1")
                .expect("tab")
                .workspace
                .persisted()
                .windows
                .iter()
                .any(|window| window.session_id.as_deref() == Some(session_id)),
            "{session_id} placeholder must survive a Monitor hold"
        );
        let session =
            gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
                .expect("load held session");
        assert!(session.restore_window_on_startup);
    }
}

/// Issue #4783 AC-1 / AC-3: a gwt restart over three Issue worktrees whose
/// execution ledgers read Completed, Blocked-by-`issue.monitor.stop`, and
/// ordinary Blocked, and Active restores only the last — with no Monitor
/// hold left in the prefs to lean on. The admitted restore is then re-judged
/// at the spawn boundary (AC-2): a hold that lands between the sweep and the
/// drain stops the spawn.
#[test]
fn startup_restore_refuses_completed_and_monitor_revoked_generations() {
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let placeholders = vec![
        (
            "agent-completed".to_string(),
            "session-completed".to_string(),
        ),
        ("agent-revoked".to_string(), "session-revoked".to_string()),
        ("agent-blocked".to_string(), "session-blocked".to_string()),
        ("agent-active".to_string(), "session-active".to_string()),
    ];
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let owner_of = |number: u64| gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number,
    };
    let mut worktrees = HashMap::new();
    for (session_id, worktree, issue) in [
        ("session-completed", "wt-completed", 4291u64),
        ("session-revoked", "wt-revoked", 4294),
        ("session-blocked", "wt-blocked", 4275),
        ("session-active", "wt-active", 4276),
    ] {
        let worktree = temp.path().join(worktree);
        fs::create_dir_all(&worktree).expect("create worktree");
        init_repo(&worktree);
        save_restore_fixture_session(
            &runtime.sessions_dir,
            session_id,
            &worktree,
            Some(&format!("native-{session_id}")),
            Some(issue),
        );
        gwt::cli::execution_state::materialize_at_launch(
            &worktree,
            gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            issue,
            session_id,
            "gwt-execute",
            false,
        )
        .expect("materialize execution record");
        worktrees.insert(session_id, (worktree, issue));
    }
    // #4291: the PR landed and the agent settled its generation.
    let (completed_wt, _) = &worktrees["session-completed"];
    assert!(matches!(
        gwt::cli::execution_state::settle(
            completed_wt,
            "session-completed",
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle completed generation"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    // #4294: the PM ran `issue.monitor.stop`, which revokes the generation
    // on the ledger. The Monitor prefs carry no hold any more (a requeue or
    // a prefs reset in between), so only the ledger can say it was stopped.
    let (revoked_wt, revoked_issue) = &worktrees["session-revoked"];
    gwt::cli::execution_state::ensure_generation_ledger(
        revoked_wt,
        owner_of(*revoked_issue),
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize owner ledger");
    assert!(matches!(
        gwt::cli::execution_state::release_revoked_launch_generation(
            revoked_wt,
            owner_of(*revoked_issue),
            "the operator revoked this launch: PR opened, holding",
        )
        .expect("release revoked generation"),
        gwt::cli::execution_state::LaunchGenerationRelease::Released { .. }
    ));
    // #4275: an ordinary Blocked generation, recoverable by reopen.
    let (blocked_wt, _) = &worktrees["session-blocked"];
    assert!(matches!(
        gwt::cli::execution_state::settle(
            blocked_wt,
            "session-blocked",
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "build failed".to_string(),
                missing_verification: Some("full matrix".to_string()),
            },
        )
        .expect("settle blocked generation"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));

    let mut admission = None;
    let logs = capture_tracing_events(|| {
        admission = Some(assess_restore_fixture(&runtime, "tab-1"));
    });
    let admission = admission.unwrap();
    let queued = admission
        .admitted
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        queued,
        vec!["session-active"],
        "terminal generations must not restore automatically"
    );
    let refusals = restore_admission_refusals(&logs);
    let completed = refusals
        .get("session-completed")
        .cloned()
        .unwrap_or_default();
    assert!(
        completed == "completed_work_retained" || completed == "terminal_work:settled_execution",
        "a Completed generation is finished Work, got {completed:?}"
    );
    assert_eq!(
        refusals.get("session-revoked").map(String::as_str),
        Some("monitor_hold:launch_revoked"),
        "{refusals:?}"
    );
    assert_eq!(
        refusals.get("session-blocked").map(String::as_str),
        Some("monitor_hold:execution_blocked"),
        "a terminal Blocked generation retains diagnostics without spawning"
    );
    // The Monitor-revoked generation is a hold, not a terminal: the
    // placeholder and the restore flag stay for `execution.reopen`.
    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .any(|window| window.session_id.as_deref() == Some("session-revoked")));
    assert!(
        gwt_agent::Session::load(&runtime.sessions_dir.join("session-revoked.toml"))
            .expect("load revoked session")
            .restore_window_on_startup
    );

    // AC-2: between the sweep and the drain the PM stops #4276 through the
    // Monitor. The queued restore is re-judged at the spawn boundary and
    // nothing spawns.
    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
    monitor.escalate_to_needs_human(
        4276,
        gwt::NeedsHumanKind::UserChoiceRequired,
        "stopped: PM held this row after the sweep",
    );
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &monitor.prefs(),
    )
    .expect("seed monitor prefs");
    let mut late_admission = None;
    let drain_logs = capture_tracing_events(|| {
        late_admission = Some(assess_restore_fixture(&runtime, "tab-1"));
    });
    assert!(
        late_admission.unwrap().admitted.is_empty(),
        "the canonical admission predicate must recheck a hold added after selection"
    );
    // The spawn boundary logs the hold; the drain then counts the missing
    // spawn as `launch_not_started`, so both reasons name this Session.
    let late = drain_logs
        .iter()
        .filter(|event| {
            event.fields.get("message").map(String::as_str) == Some("session restore refused")
                && event.fields.get("session_id").map(String::as_str) == Some("session-active")
        })
        .filter_map(|event| event.fields.get("reason").cloned())
        .collect::<Vec<_>>();
    assert!(
        late.iter()
            .any(|reason| reason.starts_with("monitor_hold:")),
        "the late refusal names the hold, got {late:?}"
    );
    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .iter()
            .any(|window| window.session_id.as_deref() == Some("session-active")),
        "a held row keeps its placeholder"
    );
}

/// Issue #4441 AC-1: the restore flag is honored on the placeholder path too.
///
/// A settled agent whose window nobody closed by hand keeps its placeholder;
/// the flag is the only durable record that the window is finished.
#[test]
fn startup_restore_honors_a_cleared_restore_flag_on_a_surviving_placeholder() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let placeholders = vec![("agent-settled".to_string(), "session-settled".to_string())];
    let tab = restore_fixture_tab("tab-1", &repo, &placeholders);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-settled",
        &temp.path().join("wt-settled"),
        Some("native-settled"),
        Some(4_441_777),
    );
    let path = runtime.sessions_dir.join("session-settled.toml");
    let mut session = gwt_agent::Session::load(&path).expect("load settled session");
    session.restore_window_on_startup = false;
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session
        .save(&runtime.sessions_dir)
        .expect("save settled session");

    let logs = capture_tracing_events(|| {
        runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    });

    assert!(runtime.pending_startup_auto_resume_sessions.is_empty());
    assert_eq!(
        restore_admission_refusals(&logs)
            .get("session-settled")
            .map(String::as_str),
        Some("window_not_open")
    );
}

/// Issue #4143 AC-3: a restore nobody asked for that dies before PTY start
/// records its reason and leaves no window behind.
/// Issue #4383 AC-5: the ledger retains its linked Issue after the pane closes.
#[test]
fn automatic_restore_launch_failure_before_pty_closes_the_window_and_records_the_reason() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("fixture project");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(&temp.path().join(".gwt"), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.tabs[0]
        .workspace
        .set_linked_issue_number("agent-1", Some(4383));
    let mut source = gwt_agent::Session::new(
        &repo,
        "main",
        gwt_agent::AgentId::Custom("missing-restore-fixture".into()),
    );
    source.id = "session-restore".into();
    source.project_state_root = Some(repo.clone());
    source.agent_session_id = Some("fixture-native-conversation".into());
    source.linked_issue_number = Some(4383);
    source.launch_command = repo.join("missing-restore-program").display().to_string();
    source.restore_window_on_startup = true;
    source.record_hook_event("Stop");
    source.record_completed_stop();
    source
        .save(&runtime.sessions_dir)
        .expect("saved restore source");
    runtime.tabs[0]
        .workspace
        .set_session_id("agent-1", Some(source.id.clone()));
    runtime
        .restore_launch_windows
        .insert(window_id.clone(), Some(source.id.clone()));
    let (executor, owner, _, detail, owner_pane_id) = drive_owner_restore_fixture(
        &mut runtime,
        &window_id,
        source,
        gwt_agent::SessionLaunchOrigin::AutomaticRestore,
    );
    let detail = detail.expect("actual owner pre-PTY failure");

    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "an automatic restore that never started a PTY must not persist a pane"
    );
    assert!(!runtime.window_lookup.contains_key(&window_id));
    // The pane is gone, so `errors.list` is the only place left that can
    // explain what happened.
    let rows = gwt_core::error_ledger::list_since(None).expect("error ledger");
    assert_eq!(rows.len(), 1, "expected one ledger row, got {rows:?}");
    assert_eq!(
        rows[0].kind,
        gwt_core::error_ledger::ErrorKind::LaunchFailure
    );
    assert!(
        rows[0].message.contains(&detail),
        "the ledger row must keep the cause after the pane is gone: {}",
        rows[0].message
    );
    assert_eq!(
        rows[0].target.window_id.as_deref(),
        Some(owner_pane_id.as_str())
    );
    assert_eq!(rows[0].target.issue, Some(4383));
    executor.block_on(owner.shutdown());
}

/// Issue #4143 AC-3: the operator's own launch keeps its error pane — that
/// diagnostic is the whole reason they are looking at the window.
#[test]
fn user_started_launch_failure_before_pty_keeps_the_error_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (owner, caller, failure) =
        super::execution_genesis_tests::genesis_pre_pty_failure_fixture(temp.path());
    let rows = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    let diagnostic = rows["canonical_rows"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"] == "failed")
        .collect::<Vec<_>>();
    assert_eq!(
        diagnostic.len(),
        1,
        "a launch the operator started keeps one diagnostic pane"
    );
    assert_eq!(diagnostic[0]["session_id"], failure.session_id);
    assert_eq!(diagnostic[0]["status"], "error");
    assert!(
        rows["panes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pane| pane["id"] != failure.id),
        "keeping an error canvas must not manufacture a PTY"
    );
    pane_control_tests::control_close(&owner, &caller);
}

/// Issue #4143 AC-6: failed restores do not breed. Two consecutive generations
/// of the same failure leave the same canvas, so the next startup has nothing
/// extra to restore.
#[test]
fn restore_launch_failures_do_not_accumulate_error_windows_across_generations() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("fixture project");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(&temp.path().join(".gwt"), vec![tab], Some("tab-1"));

    let mut persisted_after_each_generation = Vec::new();
    for generation in 0..2 {
        let raw_id = {
            let tab = runtime.tab_mut("tab-1").expect("tab");
            tab.workspace.add_window(WindowPreset::Agent).id.clone()
        };
        let window_id = combined_window_id("tab-1", &raw_id);
        runtime.register_window("tab-1", &raw_id);
        let mut source = gwt_agent::Session::new(
            &repo,
            "main",
            gwt_agent::AgentId::Custom("missing-restore-fixture".into()),
        );
        source.id = format!("failed-restore-generation-{generation}");
        source.project_state_root = Some(repo.clone());
        source.agent_session_id = Some(format!("native-failed-generation-{generation}"));
        source.launch_command = repo.join("missing-restore-program").display().to_string();
        source.restore_window_on_startup = true;
        source.record_hook_event("Stop");
        source.record_completed_stop();
        source
            .save(&runtime.sessions_dir)
            .expect("saved restore source");
        runtime.tabs[0]
            .workspace
            .set_session_id(&raw_id, Some(source.id.clone()));
        runtime
            .restore_launch_windows
            .insert(window_id.clone(), Some(source.id.clone()));
        let (executor, owner, _, error, _) = drive_owner_restore_fixture(
            &mut runtime,
            &window_id,
            source,
            gwt_agent::SessionLaunchOrigin::AutomaticRestore,
        );
        assert!(
            error.is_some(),
            "owner must observe the actual pre-PTY failure"
        );
        executor.block_on(owner.shutdown());

        persisted_after_each_generation.push(
            runtime
                .tab("tab-1")
                .expect("tab")
                .workspace
                .persisted()
                .windows
                .len(),
        );
    }

    assert_eq!(
        persisted_after_each_generation,
        vec![0, 0],
        "a failed restore must leave the canvas exactly as it found it, never one error window richer"
    );
}

/// Issue #4143 (AC-2 / AC-4 / AC-5): restore admission is affirmative. With a
/// history of 210 persisted agent windows — 150 of them backed by durably
/// closed Issues and 10 by Sessions with no resumable conversation — only the
/// 50 windows whose Work is still open may spend a PTY, the settled
/// placeholders are removed, and one summary line reports the breakdown.
#[test]
fn restore_admits_only_resumable_open_work_windows_at_history_scale() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("restore-scale");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/restore-scale",
            worktree.to_str().expect("worktree path"),
        ],
    );

    const SETTLED: usize = 150;
    const UNRESUMABLE: usize = 10;
    const OPEN: usize = 50;
    let closed_issues: Vec<u64> = (1..=SETTLED as u64).collect();
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&worktree),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            merged_issues: closed_issues.clone(),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");

    // (session id, native resume id, linked Issue)
    let mut fixtures: Vec<(String, Option<String>, Option<u64>)> = Vec::new();
    for issue in &closed_issues {
        fixtures.push((
            format!("session-closed-{issue}"),
            Some(format!("native-closed-{issue}")),
            Some(*issue),
        ));
    }
    for index in 0..UNRESUMABLE {
        fixtures.push((format!("session-unresumable-{index}"), None, None));
    }
    for index in 0..OPEN {
        fixtures.push((
            format!("session-open-{index}"),
            Some(format!("native-open-{index}")),
            None,
        ));
    }

    let mut persisted = empty_workspace_state();
    for (index, (session_id, _, _)) in fixtures.iter().enumerate() {
        let mut window = sample_window(
            &format!("codex-{index}"),
            WindowPreset::Codex,
            WindowProcessStatus::Stopped,
        );
        window.agent_id = Some("codex".to_string());
        window.session_id = Some(session_id.clone());
        // Preassigned persisted identities keep Work admission independent of
        // the capacity and language of a fresh-name catalog.
        let initial = char::from_u32(0x4e00 + index as u32).expect("fixture initial");
        window.agent_name = Some(format!("{initial}Agent"));
        persisted.windows.push(window);
    }
    persisted.next_z_index = fixtures.len() as u32 + 1;
    let tab = ProjectTabRuntime {
        id: "tab-scale".to_string(),
        title: "Restore Scale".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowRegistry::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-scale"));
    for (session_id, native_id, linked_issue) in &fixtures {
        // Each live Work owns its own worktree. Reopened AC-6 intentionally
        // refuses multiple conversations that target the same worktree.
        let session_worktree = temp.path().join(session_id);
        fs::create_dir_all(&session_worktree).expect("session worktree");
        let mut session = gwt_agent::Session::new(
            &session_worktree,
            "work/restore-scale",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.clone();
        session.agent_session_id = native_id.clone();
        session.linked_issue_number = *linked_issue;
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session.save(&runtime.sessions_dir).expect("save session");
    }

    let mut admission = None;
    let logs = capture_tracing_events(|| {
        admission = Some(assess_restore_fixture(&runtime, "tab-scale"));
    });
    let admission = admission.unwrap();
    assert_eq!(
        admission.admitted.len(),
        OPEN,
        "only readable open Work with a resumable Session is eligible at history scale"
    );
    assert!(admission
        .admitted
        .iter()
        .all(|id| id.starts_with("session-open-")));
    let windows: Vec<_> = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| {
            !window
                .session_id
                .as_ref()
                .is_some_and(|id| admission.terminal.contains(id))
        })
        .collect();
    let initials: HashSet<_> = windows
        .iter()
        .filter(|window| {
            window
                .session_id
                .as_ref()
                .is_some_and(|id| admission.admitted.contains(id))
        })
        .map(|window| window.agent_name.as_ref().unwrap().chars().next().unwrap())
        .collect();
    assert_eq!(initials.len(), OPEN);
    assert_eq!(
        windows
            .iter()
            .filter(|window| crate::runtime_support::window_is_agent_pane(window))
            .count(),
        OPEN + UNRESUMABLE,
        "canonical terminal admission selects only settled placeholders for removal"
    );
    assert!(!windows.iter().any(|window| window
        .session_id
        .as_ref()
        .is_some_and(|id| id.starts_with("session-closed-"))));
    assert_eq!(admission.terminal.len(), SETTLED);
    // AC-4: one line that explains why the canvas shrank.
    let summary = restore_admission_summary(&logs);
    assert_eq!(
        summary.fields.get("suppressed").map(String::as_str),
        Some((SETTLED + UNRESUMABLE).to_string().as_str())
    );
    let reasons = summary.fields.get("reasons").cloned().unwrap_or_default();
    assert!(
        reasons.contains(&format!("terminal_work:closed_issue={SETTLED}"))
            && reasons.contains(&format!("no_resume_session={UNRESUMABLE}")),
        "summary must break the suppression down by reason, got: {reasons}"
    );
}

/// Issue #4143 (AC-3 / AC-6): a restore that fails before its PTY starts leaves
/// no persistent Error window, so the failures cannot accumulate across
/// generations — restoring twice in a row adds nothing.
#[test]
fn restore_launch_failure_before_pty_leaves_no_error_window_across_generations() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("restore-failure");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/restore-failure",
            worktree.to_str().expect("worktree path"),
        ],
    );

    let session_ids = ["session-fail-a", "session-fail-b"];
    let mut persisted = empty_workspace_state();
    for (index, session_id) in session_ids.iter().enumerate() {
        let mut window = sample_window(
            &format!("codex-{index}"),
            WindowPreset::Codex,
            WindowProcessStatus::Stopped,
        );
        window.agent_id = Some("codex".to_string());
        window.session_id = Some((*session_id).to_string());
        persisted.windows.push(window);
    }
    persisted.next_z_index = session_ids.len() as u32 + 1;
    let tab = ProjectTabRuntime {
        id: "tab-failure".to_string(),
        title: "Restore Failure".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowRegistry::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-failure"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let _owner = project_owner_fixture(&mut runtime, "tab-failure::codex-0");
    for session_id in session_ids {
        let worktree = temp.path().join("worktrees").join(session_id);
        let branch = format!("work/{session_id}");
        run_git(
            &repo,
            &["worktree", "add", "-b", &branch, worktree.to_str().unwrap()],
        );
        let mut session = gwt_agent::Session::new(
            &worktree,
            &branch,
            gwt_agent::AgentId::Custom("missing-restore-provider".into()),
        );
        session.id = session_id.to_string();
        session.agent_session_id = Some(format!("native-{session_id}"));
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session.save(&runtime.sessions_dir).expect("save session");
    }

    let _ = runtime.restore_open_project_windows("tab-failure");
    let restored: Vec<String> = runtime
        .pending_auto_resume_sources
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        restored.len(),
        session_ids.len(),
        "both windows restore: {:?}",
        runtime.pending_auto_resume_sources
    );

    drain_owner_launch_fixtures(&mut runtime, &tasks);

    let after_first = runtime.tabs[0].workspace.persisted().windows.clone();
    assert!(
        !after_first
            .iter()
            .any(|window| window.status == WindowProcessStatus::Error),
        "a restore that died before its PTY must not persist an Error window: {after_first:?}"
    );
    assert!(
        after_first.is_empty(),
        "the failed restore windows are gone, not merely stopped: {after_first:?}"
    );
    for session_id in session_ids {
        let session = gwt_agent::Session::load_and_migrate(
            &runtime.sessions_dir.join(format!("{session_id}.toml")),
        )
        .expect("reload failed restore session");
        assert!(
            !session.restore_window_on_startup,
            "{session_id} must not be a restore candidate for the next start"
        );
    }

    // Second generation: nothing is left to restore, so nothing can be added.
    let _ = runtime.restore_open_project_windows("tab-failure");
    let after_second = runtime.tabs[0].workspace.persisted().windows.clone();
    assert!(
        after_second.is_empty(),
        "restore failures must not accumulate across generations: {after_second:?}"
    );
}

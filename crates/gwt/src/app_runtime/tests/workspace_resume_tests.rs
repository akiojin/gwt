use super::*;

#[test]
fn startup_reaper_runs_after_restore_selection_before_monitor_and_pm_dispatch() {
    let source = include_str!("../startup.rs");
    let bootstrap = source
        .split_once("pub(crate) fn bootstrap")
        .expect("bootstrap implementation")
        .1
        .split_once("pub(super) fn queue_startup_auto_resume_sessions")
        .expect("bootstrap boundary")
        .0;
    let durable_recovery = bootstrap
        .find("self.reconcile_durable_fresh_execution_launches()")
        .expect("durable recovery");
    let restore_selection = bootstrap
        .find("self.queue_startup_auto_resume_sessions")
        .expect("restore selection");
    let generation_reaper = bootstrap
        .find("self.spawn_startup_generation_reaper")
        .expect("generation reaper");
    let pm_queue = bootstrap
        .find("self.pending_startup_pm_tabs")
        .expect("PM queue");
    assert!(durable_recovery < restore_selection);
    assert!(restore_selection < generation_reaper);
    assert!(generation_reaper < pm_queue);

    let reaper = source
        .split_once("pub(super) fn reap_startup_defunct_active_generations")
        .expect("startup generation reaper")
        .1
        .split_once("pub(super) fn startup_auto_resume_ready_events")
        .expect("startup generation reaper boundary")
        .0;
    assert!(reaper.contains("classify_nonlocal_active_owner_liveness"));
}

#[test]
fn open_project_restore_resumes_paused_agent_even_after_stopped_drift() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("open-project-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/open-project-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );

    // A paused (Stopped) agent placeholder persisted in the workspace means
    // the user never closed it (closing removes it from the list).
    let mut persisted = empty_workspace_state();
    let mut agent_window =
        sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Stopped);
    agent_window.agent_id = Some("claude".to_string());
    agent_window.session_id = Some("session-open-resume".to_string());
    persisted.windows.push(agent_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-open".to_string(),
        title: "Open Resume".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-open"));

    // The backing session drifted to `Stopped` via idle timeout but was
    // never explicitly closed; it must still be restored (Issue #2942).
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/open-project-resume",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "session-open-resume".to_string();
    session.agent_session_id = Some("native-open-resume".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    let events = runtime.restore_open_project_windows("tab-open");

    assert!(
        !events.is_empty(),
        "Open Project restore should spawn the resumable agent window"
    );
    let agent_windows = runtime
        .tab("tab-open")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "the paused placeholder should be replaced by one live agent window"
    );
    assert_eq!(
        runtime.pending_auto_resume_sources.len(),
        1,
        "the source session must be tracked so it is retired on launch complete"
    );
    assert!(runtime
        .pending_auto_resume_sources
        .values()
        .any(|source| source == "session-open-resume"));
}

// SPEC-1921 Phase 65 (T335): restored Agent-family windows with exact
// provider session ids are startup auto-resumed and their stopped
// placeholders are removed across the legacy `Agent`, `Claude`, and `Codex`
// presets — the removal must not be limited to `WindowPreset::Agent`.
#[test]
fn app_runtime_startup_auto_resume_removes_stale_placeholders_across_agent_family_presets() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("family-restore");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/family-restore",
            worktree.to_str().expect("worktree path"),
        ],
    );

    let mut persisted = empty_workspace_state();
    let legacy_window = sample_window(
        "agent-legacy",
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    let mut claude_window = sample_window(
        "agent-claude",
        WindowPreset::Claude,
        WindowProcessStatus::Stopped,
    );
    claude_window.agent_id = Some("claude".to_string());
    let mut codex_window = sample_window(
        "agent-codex",
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    codex_window.agent_id = Some("codex".to_string());
    let mut legacy_window = legacy_window;
    legacy_window.session_id = Some("session-family-legacy".to_string());
    claude_window.session_id = Some("session-family-claude".to_string());
    codex_window.session_id = Some("session-family-codex".to_string());
    persisted.windows.push(legacy_window);
    persisted.windows.push(claude_window);
    persisted.windows.push(codex_window);
    persisted.next_z_index = 4;
    let tab = ProjectTabRuntime {
        id: "tab-family".to_string(),
        title: "Family Restore".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-family"));

    for (session_id, native_id, agent_id) in [
        (
            "session-family-legacy",
            "native-family-legacy",
            gwt_agent::AgentId::ClaudeCode,
        ),
        (
            "session-family-claude",
            "native-family-claude",
            gwt_agent::AgentId::ClaudeCode,
        ),
        (
            "session-family-codex",
            "native-family-codex",
            gwt_agent::AgentId::Codex,
        ),
    ] {
        let worktree = temp.path().join("worktrees").join(session_id);
        let branch = format!("work/{session_id}");
        run_git(
            &repo,
            &["worktree", "add", "-b", &branch, worktree.to_str().unwrap()],
        );
        let mut session = gwt_agent::Session::new(&worktree, &branch, agent_id);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| crate::runtime_support::window_is_agent_pane(window))
        .count();
    assert_eq!(
        agent_windows, 3,
        "each Agent-family placeholder must be replaced by exactly one resumed window; \
         a stale Claude/Codex placeholder must not survive next to its resumed window"
    );
    assert_eq!(runtime.pending_auto_resume_sources.len(), 3);
    for source in [
        "session-family-legacy",
        "session-family-claude",
        "session-family-codex",
    ] {
        assert!(
            runtime
                .pending_auto_resume_sources
                .values()
                .any(|value| value == source),
            "resumed window must track source session {source}"
        );
    }
}

// SPEC-1921 Phase 65 (T336): a restored Agent-family window whose persisted
// session has no exact provider session id must stay a stopped placeholder
// with an explicit "exact session restore is unavailable" diagnostic — and
// must never fall back to Continue / latest / new-session launches.
#[test]
fn app_runtime_startup_auto_resume_without_exact_id_keeps_placeholder_with_diagnostic() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("no-exact-id");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/no-exact-id",
            worktree.to_str().expect("worktree path"),
        ],
    );

    let mut persisted = empty_workspace_state();
    let mut codex_window = sample_window(
        "agent-noid",
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    codex_window.agent_id = Some("codex".to_string());
    codex_window.session_id = Some("session-no-exact-id".to_string());
    persisted.windows.push(codex_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-noid".to_string(),
        title: "No Exact Id".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-noid"));

    // The Codex placeholder session id is structurally unusable for exact
    // resume, so this session has no exact provider id.
    let mut session =
        gwt_agent::Session::new(&worktree, "work/no-exact-id", gwt_agent::AgentId::Codex);
    session.id = "session-no-exact-id".to_string();
    session.agent_session_id = Some("agent-session".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save session without exact id");

    runtime.seed_restored_window_details();
    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let windows = runtime.tabs[0].workspace.persisted().windows.clone();
    assert_eq!(windows.len(), 1, "no fallback window may be spawned");
    assert_eq!(windows[0].id, "agent-noid");
    assert_eq!(windows[0].status, WindowProcessStatus::Stopped);
    assert!(
        runtime.pending_auto_resume_sources.is_empty(),
        "a session without an exact provider id must not auto-resume"
    );
    assert!(
        runtime.runtimes.is_empty(),
        "no Continue/latest/new-session process may be launched as a fallback"
    );

    let detail = runtime
        .window_details
        .get(&combined_window_id("tab-noid", "agent-noid"))
        .cloned()
        .unwrap_or_default();
    assert!(
        detail.contains("Exact session restore is unavailable"),
        "placeholder must explain exact session restore is unavailable, got: {detail}"
    );
    assert!(
        !detail.contains("Restored window is paused"),
        "the generic paused message must be replaced by the exact-restore diagnostic"
    );
}

// SPEC-1921 Phase 65 (T336/T337): an exact auto-resume candidate must not be
// labeled with the generic paused-placeholder detail (it resumes as soon as
// the canvas is ready), while non-agent process windows keep the generic
// message.
#[test]
fn app_runtime_startup_auto_resume_candidate_skips_generic_paused_detail() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("candidate-detail");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/candidate-detail",
            worktree.to_str().expect("worktree path"),
        ],
    );

    let mut persisted = empty_workspace_state();
    let mut claude_window = sample_window(
        "agent-candidate",
        WindowPreset::Claude,
        WindowProcessStatus::Stopped,
    );
    claude_window.agent_id = Some("claude".to_string());
    claude_window.session_id = Some("session-candidate-detail".to_string());
    persisted.windows.push(claude_window);
    persisted.windows.push(sample_window(
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Stopped,
    ));
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-candidate".to_string(),
        title: "Candidate Detail".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-candidate"));

    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/candidate-detail",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "session-candidate-detail".to_string();
    session.agent_session_id = Some("native-candidate-detail".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.seed_restored_window_details();

    assert!(
        !runtime
            .window_details
            .contains_key(&combined_window_id("tab-candidate", "agent-candidate")),
        "an exact auto-resume candidate must not carry the generic paused detail"
    );
    let shell_detail = runtime
        .window_details
        .get(&combined_window_id("tab-candidate", "shell-1"))
        .cloned()
        .unwrap_or_default();
    assert!(
        shell_detail.contains("Restored window is paused"),
        "non-agent process windows keep the generic paused message, got: {shell_detail}"
    );
}

#[test]
fn app_runtime_startup_auto_resume_uses_centered_stack_bounds() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("centered-stack");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/centered-stack",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    for (index, native_session_id) in ["native-stack-one", "native-stack-two", "native-stack-three"]
        .into_iter()
        .enumerate()
    {
        let worktree = temp.path().join("worktrees").join(native_session_id);
        let branch = format!("work/{native_session_id}");
        run_git(
            &repo,
            &["worktree", "add", "-b", &branch, worktree.to_str().unwrap()],
        );
        let mut session = gwt_agent::Session::new(&worktree, &branch, gwt_agent::AgentId::Codex);
        session.id = format!("session-centered-stack-{index}");
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session.last_activity_at = chrono::Utc::now() - chrono::Duration::seconds(index as i64);
        session.updated_at = session.last_activity_at;
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let geometries = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .map(|window| window.geometry.clone())
        .collect::<Vec<_>>();
    assert_eq!(geometries.len(), 3);
    assert_eq!(
        geometries
            .iter()
            .map(|geometry| (geometry.x, geometry.y, geometry.width, geometry.height))
            .collect::<Vec<_>>(),
        vec![
            (32.0, 26.0, 1280.0, 800.0),
            (60.0, 50.0, 1280.0, 800.0),
            (88.0, 74.0, 1280.0, 800.0),
        ],
        "restored agent windows should form a stack centered in the startup canvas"
    );
}

#[test]
fn app_runtime_startup_auto_resume_excludes_closed_stopped_windows() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("restore-flag");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/restore-flag",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    for (session_id, native_session_id, restore_window_on_startup, stopped) in [
        ("session-open-window", "native-open-window", true, false),
        ("session-closed-window", "native-closed-window", false, true),
    ] {
        let mut session =
            gwt_agent::Session::new(&worktree, "work/restore-flag", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = restore_window_on_startup;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        if stopped {
            session.update_status(gwt_agent::AgentStatus::Stopped);
        }
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let resumed_sources = runtime
        .pending_auto_resume_sources
        .values()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        resumed_sources,
        std::collections::HashSet::from(["session-open-window".to_string()])
    );
}

#[test]
fn app_runtime_startup_recovery_preserves_unchanged_session_file_identity() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(&worktree, "work/no-op", gwt_agent::AgentId::Codex);
    session.id = "session-no-op-startup".to_string();
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save no-op session");

    let session_path = runtime.sessions_dir.join("session-no-op-startup.toml");
    let mut session_file = OpenOptions::new()
        .append(true)
        .open(&session_path)
        .expect("open no-op session");
    session_file
        .write_all(b"\n# preserve-no-op-session\n")
        .expect("append no-op sentinel");
    drop(session_file);
    let before = fs::read(&session_path).expect("read no-op session before startup recovery");
    let identity_probe = runtime.sessions_dir.join("session-no-op-startup.identity");
    fs::hard_link(&session_path, &identity_probe).expect("link no-op session identity probe");

    let loaded = runtime.load_recovery_sessions();

    let after = fs::read(&session_path).expect("read no-op session after startup recovery");
    let mut probe_file = OpenOptions::new()
        .append(true)
        .open(&identity_probe)
        .expect("open identity probe");
    probe_file
        .write_all(b"# same-file-identity\n")
        .expect("append identity probe");
    drop(probe_file);
    let original_after_probe =
        fs::read(&session_path).expect("read original after identity probe append");

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, session.id);
    assert_eq!(
        after, before,
        "current no-op Session must remain byte-identical"
    );
    assert!(
        original_after_probe.ends_with(b"# same-file-identity\n"),
        "startup recovery must not replace the no-op Session inode"
    );
}

#[test]
fn app_runtime_startup_recovery_preserves_already_interrupted_session_file_identity() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session =
        gwt_agent::Session::new(&worktree, "work/interrupted", gwt_agent::AgentId::Codex);
    session.id = "session-already-interrupted".to_string();
    session.record_hook_event("UserPromptSubmit");
    session.update_status(gwt_agent::AgentStatus::Interrupted);
    session
        .save(&runtime.sessions_dir)
        .expect("save interrupted session");

    let session_path = runtime
        .sessions_dir
        .join("session-already-interrupted.toml");
    let before = fs::read(&session_path).expect("read interrupted session before recovery");
    let identity_probe = runtime
        .sessions_dir
        .join("session-already-interrupted.identity");
    fs::hard_link(&session_path, &identity_probe).expect("link interrupted identity probe");

    let loaded = runtime.load_recovery_sessions();

    let after = fs::read(&session_path).expect("read interrupted session after recovery");
    let mut probe_file = OpenOptions::new()
        .append(true)
        .open(&identity_probe)
        .expect("open interrupted identity probe");
    probe_file
        .write_all(b"# same-interrupted-file-identity\n")
        .expect("append interrupted identity probe");
    drop(probe_file);
    let original_after_probe =
        fs::read(&session_path).expect("read interrupted original after probe append");

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, session.id);
    assert_eq!(
        after, before,
        "already Interrupted Session must remain byte-identical"
    );
    assert!(
        original_after_probe.ends_with(b"# same-interrupted-file-identity\n"),
        "startup recovery must not replace an already Interrupted Session inode"
    );
}

#[test]
fn app_runtime_startup_recovery_persists_legacy_migration_and_skips_malformed() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(&worktree, "work/legacy", gwt_agent::AgentId::Codex);
    session.id = "session-legacy-startup".to_string();
    session.schema_version = 2;
    session.status = gwt_agent::AgentStatus::Running;
    session.last_hook_event = None;
    session.last_hook_event_at = None;
    session.last_completed_stop_at = None;
    session
        .save(&runtime.sessions_dir)
        .expect("save legacy session");
    fs::write(
        runtime.sessions_dir.join("malformed-session.toml"),
        "not = [valid toml",
    )
    .expect("write malformed session");

    let loaded = runtime.load_recovery_sessions();

    assert_eq!(loaded.len(), 1, "malformed Session must remain isolated");
    assert_eq!(
        loaded[0].schema_version,
        gwt_agent::Session::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(loaded[0].status, gwt_agent::AgentStatus::Interrupted);
    let persisted =
        gwt_agent::Session::load(&runtime.sessions_dir.join("session-legacy-startup.toml"))
            .expect("load migrated Session without applying another migration");
    assert_eq!(
        persisted.schema_version,
        gwt_agent::Session::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(persisted.status, gwt_agent::AgentStatus::Interrupted);
}

/// Issue #5025 AC-3: startup uses shared classification and failed-read caching.
#[test]
fn app_runtime_startup_recovery_classifies_preferences_and_caches_failed_candidates() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    // Resolve the setup's empty background load before writing failures; it
    // must not prime the shared failure cache outside the tracing capture.
    let _ = runtime.launch_wizard_cache.agent_preferences();
    let mut session =
        gwt_agent::Session::new(temp.path(), "work/startup", gwt_agent::AgentId::Codex);
    session.id = "valid-startup".to_string();
    session.status = gwt_agent::AgentStatus::Stopped;
    session
        .save(&runtime.sessions_dir)
        .expect("save valid Session");
    for (name, content) in [
        ("window-state.toml", "display_mode = 'grid'\n"),
        ("legacy-launch-prefs.toml", "last_branch = 'main'\n"),
        (".writer-temporary.toml", "not = [valid toml"),
        ("malformed-session.toml", "not = [valid toml"),
    ] {
        fs::write(runtime.sessions_dir.join(name), content).expect("write ledger fixture");
    }
    let mut missing_agent = toml::Value::try_from(&session).expect("Session TOML value");
    let table = missing_agent.as_table_mut().expect("Session table");
    table.insert("id".to_string(), "missing-agent".into());
    table.remove("agent_id");
    fs::write(
        runtime.sessions_dir.join("missing-agent.toml"),
        toml::to_string(&missing_agent).expect("serialize missing-agent fixture"),
    )
    .expect("write missing-agent fixture");

    let events = capture_tracing_events(|| {
        for _ in 0..2 {
            let loaded = runtime.load_recovery_sessions();
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].id, session.id);
        }
    });
    let warnings = events
        .iter()
        .filter(|event| {
            event
                .fields
                .get("message")
                .is_some_and(|message| message.starts_with("Cannot load session"))
        })
        .collect::<Vec<_>>();
    assert!(
        warnings.len() <= 2,
        "preferences/hidden temporary must not reach typed Session reads, and unchanged failed candidates must not flood warnings; captured={events:?}"
    );
    // Other background readers can emit a failure outside this thread-local
    // capture. Candidate parse-count tests prove unchanged failures are not
    // reread; every diagnostic captured here must still belong to a bad file.
    assert!(warnings
        .iter()
        .all(|event| event.fields.get("path").is_some_and(|path| {
            path.ends_with("malformed-session.toml") || path.ends_with("missing-agent.toml")
        })));
    for name in ["malformed-session.toml", "missing-agent.toml"] {
        assert!(
            warnings
                .iter()
                .filter(|event| event
                    .fields
                    .get("path")
                    .is_some_and(|path| path.ends_with(name)))
                .count()
                <= 1,
            "failed candidate {name} must not warn repeatedly across startup reads"
        );
    }
}

/// Issue #4377 (AC-1 / AC-3): startup reads only the Sessions it may restore.
/// A stale Session file is neither locked nor parsed on the startup path; the
/// blocking worker applies the same Interrupted judgement afterwards. A
/// placeholder-referenced Session, or any Session of an update-resumed
/// project, is still read regardless of age.
#[test]
fn app_runtime_startup_recovery_reads_only_restore_candidates_and_defers_the_rest() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let mut persisted = empty_workspace_state();
    let mut placeholder = sample_window(
        "agent-placeholder",
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    placeholder.session_id = Some("session-placeholder".to_string());
    persisted.windows.push(placeholder);
    persisted.next_z_index = 2;
    let mut tab = sample_project_tab("tab-repo", "Repo", worktree.clone(), ProjectKind::Git, &[]);
    tab.workspace = WindowCanvasState::from_persisted(persisted);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let age = |session_id: &str| {
        fs::File::options()
            .write(true)
            .open(runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("open session file")
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60))
            .expect("age session file");
    };
    for session_id in ["session-fresh", "session-stale", "session-placeholder"] {
        let mut session =
            gwt_agent::Session::new(&worktree, "work/recovery", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.record_hook_event("UserPromptSubmit");
        session.status = gwt_agent::AgentStatus::Running;
        session.save(&runtime.sessions_dir).expect("save session");
    }
    age("session-stale");
    age("session-placeholder");
    let stale_path = runtime.sessions_dir.join("session-stale.toml");
    // `Session::save` takes the per-Session lock too; drop the setup's lock
    // file so the assertion below sees only what the startup read does.
    fs::remove_file(runtime.sessions_dir.join(".session-stale.lock")).expect("drop setup lock");

    let mut loaded = runtime
        .load_recovery_sessions()
        .into_iter()
        .map(|session| (session.id, session.status))
        .collect::<Vec<_>>();
    loaded.sort_by(|left, right| left.0.cmp(&right.0));

    assert_eq!(
        loaded,
        vec![
            (
                "session-fresh".to_string(),
                gwt_agent::AgentStatus::Interrupted
            ),
            (
                "session-placeholder".to_string(),
                gwt_agent::AgentStatus::Interrupted
            ),
        ]
    );
    assert!(
        !runtime.sessions_dir.join(".session-stale.lock").exists(),
        "a stale Session must not be locked or parsed on the startup path"
    );
    assert_eq!(
        gwt_agent::Session::load(&stale_path).expect("stale").status,
        gwt_agent::AgentStatus::Running
    );
    let queued = std::mem::take(
        &mut *tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for task in queued {
        task();
    }
    assert_eq!(
        gwt_agent::Session::load(&stale_path).expect("stale").status,
        gwt_agent::AgentStatus::Interrupted,
        "the deferred sweep must apply the same Interrupted judgement"
    );

    // Issue #4038: an update-resumed project bypasses the freshness gate, so
    // its old Sessions are restore candidates again.
    age("session-stale");
    runtime.update_resume_tab_ids.insert("tab-repo".to_string());
    assert!(runtime
        .load_recovery_sessions()
        .iter()
        .any(|session| session.id == "session-stale"));
}

/// Issue #4730 AC-3: historical rows cannot multiply synchronous Git work.
#[test]
fn startup_restore_defers_1500_old_sessions_without_git_spawns() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    fs::create_dir_all(&runtime.sessions_dir).unwrap();
    let modified = std::time::SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60);
    for index in 0..1500 {
        let mut session = gwt_agent::Session::new(&repo, "work/history", gwt_agent::AgentId::Codex);
        session.id = format!("history-{index}");
        session.last_activity_at = chrono::Utc::now() - chrono::Duration::days(3);
        let path = runtime.sessions_dir.join(format!("{}.toml", session.id));
        fs::write(&path, toml::to_string(&session).unwrap()).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }
    let git_spawns = gwt_core::process::thread_git_spawn_count();
    runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    assert_eq!(gwt_core::process::thread_git_spawn_count() - git_spawns, 0);
    assert!(runtime.pending_startup_auto_resume_sessions.is_empty());
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "one deferred recovery sweep"
    );
    assert!(
        fs::read_dir(&runtime.sessions_dir).unwrap().all(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "toml")
        }),
        "historical Sessions must not be locked and parsed before the deferred sweep"
    );
}

#[test]
fn startup_restore_update_marker_1500_sessions_bounds_git_spawns() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktrees: Vec<_> = (0..6)
        .map(|index| {
            let path = temp.path().join(format!("linked-{index}"));
            run_git(
                &repo,
                &["worktree", "add", "--detach", path.to_str().unwrap()],
            );
            path
        })
        .collect();
    let tab = sample_project_tab("tab-repo", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let (spawner, _tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.update_resume_tab_ids.insert("tab-repo".to_string());
    fs::create_dir_all(&runtime.sessions_dir).unwrap();
    for index in 0..1500 {
        let mut session = gwt_agent::Session::new(
            &worktrees[index % worktrees.len()],
            "history",
            gwt_agent::AgentId::Codex,
        );
        session.id = format!("update-history-{index}");
        session.last_activity_at = chrono::Utc::now() - chrono::Duration::days(3);
        fs::write(
            runtime.sessions_dir.join(format!("{}.toml", session.id)),
            toml::to_string(&session).unwrap(),
        )
        .unwrap();
    }
    let before = gwt_core::process::thread_git_spawn_count();
    let started = Instant::now();
    runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    let spawns = gwt_core::process::thread_git_spawn_count() - before;
    eprintln!(
        "update restore: sessions=1500 git_spawns={spawns} elapsed={:?}",
        started.elapsed()
    );
    assert!(
        spawns <= 5,
        "update restore spawned {spawns} Git processes; budget is 5"
    );
    assert!(runtime.pending_startup_auto_resume_sessions.is_empty());
}

#[test]
fn app_runtime_startup_auto_resume_includes_legacy_non_stopped_sessions() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("legacy-restore");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/legacy-restore",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    let mut session =
        gwt_agent::Session::new(&worktree, "work/legacy-restore", gwt_agent::AgentId::Codex);
    session.id = "session-legacy-open-window".to_string();
    session.agent_session_id = Some("native-legacy-open-window".to_string());
    session.restore_window_on_startup = false;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save legacy open session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let resumed_sources = runtime
        .pending_auto_resume_sources
        .values()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
            resumed_sources,
            std::collections::HashSet::from(["session-legacy-open-window".to_string()]),
            "legacy sessions without the new restore flag should still restore when they were not explicitly stopped"
        );
}

#[test]
fn app_runtime_close_agent_window_clears_startup_restore_eligibility() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some(tab_id));
    let mut session =
        gwt_agent::Session::new(&worktree, "work/restore-flag", gwt_agent::AgentId::Codex);
    session.id = "session-close-clears-restore".to_string();
    session.restore_window_on_startup = true;
    session
        .save(&runtime.sessions_dir)
        .expect("save active session");
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load(&runtime.sessions_dir);
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: session.id.clone(),
            agent_id: "codex".to_string(),
            branch_name: "work/restore-flag".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.close_window_events(&window_id);
    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("close finalizer");
    finalizer();

    let loaded = gwt_agent::Session::load(
        &runtime
            .sessions_dir
            .join("session-close-clears-restore.toml"),
    )
    .expect("load session");
    assert!(!loaded.restore_window_on_startup);
}

#[test]
fn app_runtime_bootstrap_auto_resumes_same_repo_worktree_session_from_restored_project_tab() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("same-repo-session");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/same-repo-session",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/same-repo-session",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-same-repo-worktree".to_string();
    session.agent_session_id = Some("native-same-repo-worktree".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert_eq!(
        runtime.tabs.len(),
        1,
        "bootstrap should keep the restored project tab instead of opening a hidden worktree tab"
    );
    assert!(same_worktree_path(&runtime.tabs[0].project_root, &repo));
    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
            agent_windows, 1,
            "resumable sessions from local worktrees in the restored repo should restart inside the project tab"
        );
}

#[test]
fn app_runtime_bootstrap_ignores_same_repo_worktree_session_without_lifecycle() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("no-lifecycle-session");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/no-lifecycle-session",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/no-lifecycle-session",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-same-repo-no-lifecycle".to_string();
    session.agent_session_id = Some("native-no-lifecycle".to_string());
    session.restore_window_on_startup = true;
    session.update_status(gwt_agent::AgentStatus::Running);
    session
        .save(&runtime.sessions_dir)
        .expect("save stale session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
            agent_windows, 0,
            "same-repo fallback must still require lifecycle evidence so old session history does not mass launch"
        );
}

#[test]
fn app_runtime_bootstrap_ignores_same_repo_worktree_session_with_placeholder_resume_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("placeholder-session");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/placeholder-session",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/placeholder-session",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-same-repo-placeholder".to_string();
    session.agent_session_id = Some("agent-session".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save placeholder session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 0,
        "placeholder Codex hook ids must not launch `codex resume agent-session`"
    );
}

#[test]
fn app_runtime_bootstrap_does_not_auto_resume_sessions_outside_restored_tabs() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("unlisted-auto-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/unlisted-auto-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/unlisted-auto-resume",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-unlisted-auto".to_string();
    session.agent_session_id = Some("native-unlisted-auto".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert!(
            runtime.tabs.is_empty(),
            "bootstrap must not open project tabs from old session TOMLs that were not restored from session.json"
        );
    assert!(
            runtime.pending_auto_resume_sources.is_empty(),
            "unlisted sessions must remain manual resume candidates instead of launching hidden agent windows"
        );
}

#[test]
fn app_runtime_bootstrap_auto_resume_dedupes_and_skips_stale_without_count_cap() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("auto-resume-guard");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/auto-resume-guard",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-worktree",
        "Worktree",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-worktree"));
    let now = chrono::Utc::now();
    let cases = [
        ("session-fresh-1", "native-one", 1_i64),
        ("session-duplicate-native-one", "native-one", 2_i64),
        ("session-fresh-2", "native-two", 3_i64),
        ("session-fresh-3", "native-three", 4_i64),
        ("session-fresh-4", "native-four", 5_i64),
        ("session-stale", "native-stale", 60 * 60 * 48_i64),
    ];
    for (session_id, native_session_id, age_secs) in cases {
        let worktree = temp.path().join("worktrees").join(session_id);
        let branch = format!("work/{session_id}");
        run_git(
            &repo,
            &["worktree", "add", "-b", &branch, worktree.to_str().unwrap()],
        );
        let mut session = gwt_agent::Session::new(&worktree, &branch, gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session.last_activity_at = now - chrono::Duration::seconds(age_secs);
        session.updated_at = session.last_activity_at;
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert_eq!(
        runtime.pending_auto_resume_sources.len(),
        4,
        "startup auto-resume must restore every fresh unique exact-resumable session"
    );
    let resumed_sources = runtime
        .pending_auto_resume_sources
        .values()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    assert!(
        !resumed_sources.contains("session-duplicate-native-one"),
        "duplicate native agent session ids must not launch twice"
    );
    assert!(
        !resumed_sources.contains("session-stale"),
        "stale persisted sessions must stay available for manual resume instead of auto-launching"
    );
    assert!(
        resumed_sources.contains("session-fresh-4"),
        "startup auto-resume must not drop fresh unique sessions due to an arbitrary count cap"
    );
}

#[test]
fn app_runtime_resume_workspace_journal_populates_quick_start_entries_from_prior_sessions() {
    // SPEC-2359 US-44 (Issue #2757) follow-on: when the user clicks
    // `Resume` on a Workspace journal card whose branch already has a
    // prior session on disk, the Launch Wizard should expose that prior
    // session through the Quick Start panel immediately. Without this,
    // the Quick Start panel only fills after the user completes the
    // runtime resolution step, which forces a multi-click resume path
    // even though the resumable session metadata is already known.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "work/20260518-resume-qs";
    run_git(&repo, &["branch", branch]);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .expect("save projection");
    append_workspace_resume_journal(
        &repo,
        "journal-quickstart",
        temp.path().join("work").join("20260518-resume-qs"),
        "SPEC-2359",
        "Resume with prior Codex session available",
    );

    // Pre-seed a Session toml that matches the resumable branch so the
    // wizard cache exposes it through Quick Start entries.
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");
    let mut session = gwt_agent::Session::new(&repo, branch, gwt_agent::AgentId::Codex);
    session.display_name = "Codex".to_string();
    session.agent_session_id = Some("prior-codex-uuid".to_string());
    session.tool_version = Some("installed".to_string());
    session.save(&sessions_dir).expect("save session toml");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Journal,
            journal_id: Some("journal-quickstart".to_string()),
        },
    );

    let session_ref = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard opened");
    let view = session_ref.wizard.view();
    assert_eq!(view.branch_name, branch);
    assert!(
            !view.quick_start_entries.is_empty(),
            "Resume must pre-populate Quick Start entries so the prior session is immediately resumable; saw empty Quick Start panel"
        );
    assert!(
            view.quick_start_entries
                .iter()
                .any(|entry| entry.resume_session_id.as_deref() == Some("prior-codex-uuid")),
            "Expected the prior Codex session to appear in Quick Start entries with its resume_session_id surfaced"
        );
}

#[test]
fn app_runtime_resume_workspace_journal_falls_back_to_new_work_branch_when_branch_is_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .expect("save projection");
    append_workspace_resume_journal(
        &repo,
        "journal-new-work",
        temp.path().join("work").join("20260507-0002"),
        "Issue #2359",
        "Carry this suspended context into a new work branch.",
    );
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Journal,
            journal_id: Some("journal-new-work".to_string()),
        },
    );

    let session = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Start Work");
    assert!(view.branch_name.starts_with("work/"));
    assert_ne!(view.branch_name, "work/20260507-0002");
    assert_eq!(view.linked_issue_number, Some(2359));
    let context = session
        .workspace_resume_context
        .as_ref()
        .expect("workspace resume context");
    assert_eq!(context.owner.as_deref(), Some("Issue #2359"));
    assert_eq!(
        context.summary.as_deref(),
        Some("Carry this suspended context into a new work branch.")
    );
}

#[test]
fn app_runtime_resume_workspace_current_ignores_idle_stale_git_details() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let stale_branch = "work/20260507-stale";
    run_git(&repo, &["branch", stale_branch]);
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "Stale Workspace".to_string();
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Idle;
    projection.status_text = "No active work".to_string();
    projection.summary = Some("Old work should not be resumed".to_string());
    projection.owner = Some("Issue #2359".to_string());
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some(stale_branch.to_string()),
        worktree_path: Some(temp.path().join("work/20260507-stale")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Current,
            journal_id: None,
        },
    );

    let session = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Start Work");
    assert!(view.branch_name.starts_with("work/"));
    assert_ne!(view.branch_name, stale_branch);
    let context = session
        .workspace_resume_context
        .as_ref()
        .expect("workspace resume context");
    assert_eq!(context.title.as_deref(), Some("Repo Work"));
    assert_eq!(context.owner, None);
    assert_eq!(context.summary, None);
}

#[test]
fn app_runtime_resume_workspace_journal_derives_feature_branch_under_work_named_repo_parent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("work").join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "feature/resume-existing";
    run_git(&repo, &["branch", branch]);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .expect("save projection");
    append_workspace_resume_journal(
        &repo,
        "journal-feature",
        temp.path()
            .join("work")
            .join("feature")
            .join("resume-existing"),
        "Issue #2359",
        "Resume a non-work branch from a deleted worktree path.",
    );
    assert_eq!(
        super::super::workspace_resume_branch_from_journal_project_root(
            &temp
                .path()
                .join("work")
                .join("feature")
                .join("resume-existing"),
            &repo
        )
        .as_deref(),
        Some(branch)
    );
    assert!(super::super::workspace_resume_branch_exists(&repo, branch));
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Journal,
            journal_id: Some("journal-feature".to_string()),
        },
    );

    let session = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Launch Agent");
    assert_eq!(view.branch_name, branch);
}

#[test]
fn app_runtime_active_work_projection_exposes_done_workspace_cleanup_candidate() {
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
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260507-0200".to_string()),
        worktree_path: Some(repo.join("work/20260507-0200")),
        base_branch: Some("origin/main".to_string()),
        pr_number: Some(2525),
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_live_process_branches
        .insert(repo.clone(), HashSet::new());

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let candidate = view.cleanup_candidate.expect("cleanup candidate");

    assert_eq!(candidate.branch, "work/20260507-0200");
    assert_eq!(candidate.reason, "workspace_done");
    assert!(!candidate.default_delete_remote);
}

#[test]
fn app_runtime_active_work_projection_does_not_spawn_git_for_cleanup_candidate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("create fake bin");
    let fake_git = write_fake_git_recorder(&fake_bin);
    let git_log = temp.path().join("git-invocations.log");
    let _path = prepend_tool_parent_to_path(&fake_git);
    let _git_log = ScopedEnvVar::set("GWT_FAKE_GIT_LOG", &git_log);
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260507-0200".to_string()),
        worktree_path: Some(repo.join("work/20260507-0200")),
        base_branch: Some("origin/main".to_string()),
        pr_number: Some(2525),
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_live_process_branches
        .insert(repo.clone(), HashSet::new());

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert!(view.cleanup_candidate.is_some());
    let invocations = fs::read_to_string(&git_log).unwrap_or_default();
    assert!(
            invocations.trim().is_empty(),
            "active-work projection must not spawn git on the GUI hot path; invocations:\n{invocations}"
    );
}

#[test]
fn active_work_projection_many_workspaces_does_not_probe_dirty_worktrees() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    for index in 0..64 {
        let branch = format!("work/process-free-{index}");
        let worktree_path = repo.join("work").join(index.to_string());
        fs::create_dir_all(&worktree_path).expect("create worktree fixture");
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            format!("work-process-free-{index}"),
            chrono::Utc::now(),
        );
        event.title = Some(format!("Process-free work {index}"));
        if index == 0 {
            let session_id = "session-process-free-projection";
            event.agent_session_id = Some(session_id.to_string());
            event.agent_id = Some("codex".to_string());
            let mut session =
                gwt_agent::Session::new(&worktree_path, &branch, gwt_agent::AgentId::Codex);
            session.id = session_id.to_string();
            session.project_state_root = Some(repo.clone());
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save projection Session fixture");
        }
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(branch),
                worktree_path: Some(worktree_path),
                pr_number: Some(3_000 + index),
                pr_url: None,
                pr_state: Some("MERGED".to_string()),
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
            .expect("record work");
    }

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_dirty_branches
        .insert(repo.clone(), HashSet::new());
    runtime
        .work_live_process_branches
        .insert(repo.clone(), HashSet::new());
    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("create fake bin");
    let fake_git = write_fake_git_recorder(&fake_bin);
    let git_log = temp.path().join("git-invocations.log");
    let _path = prepend_tool_parent_to_path(&fake_git);
    let _git_log = ScopedEnvVar::set("GWT_FAKE_GIT_LOG", &git_log);

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_works.len(), 64);
    assert!(
        view.active_works
            .iter()
            .all(|work| work.cleanup_candidate.is_some()),
        "clean background caches keep every merged row cleanup-ready"
    );
    let diagnoses = view
        .active_works
        .iter()
        .flat_map(|workspace| workspace.works.iter())
        .filter_map(|work| work.execution_diagnosis.as_ref())
        .collect::<Vec<_>>();
    assert!(
        !diagnoses.is_empty(),
        "projection keeps durable diagnosis facts"
    );
    for diagnosis in diagnoses {
        for protected in [
            "execution.continue",
            "execution.repair",
            "execution.adopt",
            "execution.reopen",
            "workspace.update",
            "workspace.ensure",
        ] {
            assert!(
                !diagnosis
                    .available_recoveries
                    .iter()
                    .any(|operation| operation == protected),
                "projection must not advertise unvalidated protected recovery {protected}"
            );
        }
    }
    let invocations = fs::read_to_string(&git_log).unwrap_or_default();
    assert!(
        invocations.trim().is_empty(),
        "projection must not spawn Git on the GUI event loop; invocations:\n{invocations}"
    );
}

#[test]
fn active_work_projection_source_has_no_live_process_scan_helper() {
    let source = include_str!("../workspace_views.rs");
    assert!(
        !source.contains("fn live_process_worktree_paths_for_cleanup("),
        "all-OS-process enumeration must run in the background merge scan, not projection"
    );
}

/// Issue #3611 AC-1/AC-2/AC-3: a Session whose worktree was deleted used to
/// fall through `session_exact_resume_materializable` into
/// `git rev-parse --git-common-dir` + `git show-ref` **per Session**, on the
/// GUI event-loop thread. `pane.*` is served from that same single-threaded
/// loop, so its response latency *is* the projection's occupancy (#3510: 6ms
/// idle, >2s during a scan). At this fixture's scale the old path meant ~384
/// short-lived Git processes (~6s) per build.
///
/// Both assertions target that cost: the spawn count must stay flat at zero
/// however many Sessions exist, and the build must finish inside a budget well
/// below the pre-fix cost while leaving ~40x headroom over the measured
/// process-free build (~45ms), so a loaded machine cannot flake it.
#[test]
fn active_work_projection_with_missing_worktrees_does_not_spawn_git_per_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    for index in 0..128 {
        let branch = format!("work/missing-worktree-{index}");
        // Deliberately never created: this is the historical Session whose
        // worktree is gone but whose branch may still exist.
        let worktree_path = repo.join("work").join(index.to_string());
        let session_id = format!("session-missing-worktree-{index}");
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            format!("work-missing-worktree-{index}"),
            chrono::Utc::now(),
        );
        event.title = Some(format!("Missing worktree work {index}"));
        event.agent_session_id = Some(session_id.clone());
        event.agent_id = Some("codex".to_string());
        let mut session =
            gwt_agent::Session::new(&worktree_path, &branch, gwt_agent::AgentId::Codex);
        session.id = session_id;
        session.agent_session_id = Some(format!("conv-missing-worktree-{index}"));
        session.project_state_root = Some(repo.clone());
        session
            .save(&sessions_dir)
            .expect("save projection Session fixture");
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(branch),
                worktree_path: Some(worktree_path),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
            .expect("record work");
    }

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_dirty_branches
        .insert(repo.clone(), HashSet::new());
    runtime
        .work_live_process_branches
        .insert(repo.clone(), HashSet::new());
    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("create fake bin");
    let fake_git = write_fake_git_recorder(&fake_bin);
    let git_log = temp.path().join("git-invocations.log");
    let _path = prepend_tool_parent_to_path(&fake_git);
    let _git_log = ScopedEnvVar::set("GWT_FAKE_GIT_LOG", &git_log);

    let started = Instant::now();
    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let elapsed = started.elapsed();

    assert_eq!(view.active_works.len(), 128);
    assert!(
        view.active_works
            .iter()
            .flat_map(|workspace| workspace.agents.iter())
            .any(|agent| !agent.sessions.is_empty()),
        "fixture must actually render Session rows, otherwise the probe is untested"
    );
    let invocations = fs::read_to_string(&git_log).unwrap_or_default();
    assert!(
        invocations.trim().is_empty(),
        "Session resumability must resolve from the background ref snapshot, \
         never from a per-Session Git spawn on the event loop; invocations:\n{invocations}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "128 Sessions held the event loop for {elapsed:?}; \
         `pane.*` cannot answer while this runs"
    );
}

/// Issue #3611 AC-2: resumability answers come from the published branch-ref
/// snapshot, so the verdict is exact once the background scan has run — a
/// Session whose branch survives stays resumable, one whose branch is gone does
/// not — without either case touching a Git process.
#[test]
fn active_work_projection_resumability_follows_published_branch_refs() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    let live_branch = "work/ref-snapshot-live";
    let gone_branch = "work/ref-snapshot-gone";
    for (index, branch) in [live_branch, gone_branch].into_iter().enumerate() {
        let session_id = format!("session-ref-snapshot-{index}");
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            format!("work-ref-snapshot-{index}"),
            chrono::Utc::now(),
        );
        event.title = Some(format!("Ref snapshot work {index}"));
        event.agent_session_id = Some(session_id.clone());
        event.agent_id = Some("codex".to_string());
        let worktree_path = repo.join("work").join(format!("ref-snapshot-{index}"));
        let mut session =
            gwt_agent::Session::new(&worktree_path, branch, gwt_agent::AgentId::Codex);
        session.id = session_id;
        session.agent_session_id = Some(format!("conv-ref-snapshot-{index}"));
        session.project_state_root = Some(repo.clone());
        session
            .save(&sessions_dir)
            .expect("save projection Session fixture");
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(branch.to_string()),
                worktree_path: Some(worktree_path),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
            .expect("record work");
    }

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.apply_work_merge_status(
        &repo,
        HashMap::new(),
        HashMap::new(),
        HashSet::new(),
        HashSet::new(),
        Some(HashSet::from([
            live_branch.to_string(),
            format!("origin/{live_branch}"),
            "main".to_string(),
        ])),
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    let resumable_by_branch: HashMap<String, bool> = view
        .active_works
        .iter()
        .filter_map(|workspace| {
            let branch = workspace.branch.clone()?;
            let resumable = workspace
                .agents
                .iter()
                .flat_map(|agent| agent.sessions.iter())
                .any(|session| session.resumable);
            Some((branch, resumable))
        })
        .collect();

    assert_eq!(
        resumable_by_branch.get(live_branch),
        Some(&true),
        "a Session whose branch is in the snapshot can re-materialize its worktree"
    );
    assert_eq!(
        resumable_by_branch.get(gone_branch),
        Some(&false),
        "a Session with neither worktree nor branch is history-only"
    );
}

#[test]
fn workspace_cleanup_failure_does_not_emit_done_work_item() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let branch = "work/missing";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some(branch.to_string()),
        worktree_path: Some(repo.join("work/missing")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(2828),
        pr_state: Some("MERGED".to_string()),
        pr_url: Some("https://github.com/akiojin/gwt/pull/2828".to_string()),
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    let work_item_id = projection.id.clone();
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        &work_item_id,
        chrono::Utc::now(),
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
        .expect("record start");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let immediate_events = runtime.run_workspace_cleanup_events(
        &runtime.test_context(),
        "client-1",
        branch,
        false,
        false,
        "cleanup-op-1",
    );

    assert!(immediate_events.is_empty());
    wait_for_recorded_event("workspace cleanup failure", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ProjectDispatch { events: outbound_events, .. }
                    if outbound_events.iter().any(|outbound| matches!(
                        outbound.event,
                        BackendEvent::BranchCleanupResult { .. }
                            | BackendEvent::BranchError { .. }
                    ))
            )
        })
    });
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load work items")
        .expect("work items");
    let item = work_items
        .work_items
        .iter()
        .find(|item| item.id == work_item_id)
        .expect("work item");

    assert!(
        !item
            .events
            .iter()
            .any(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Done),
        "failed cleanup must not mark the Workspace work item done"
    );
}

#[test]
fn app_runtime_active_work_projection_exposes_saved_pr_metadata_without_live_agents() {
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
    projection.title = "Active Work PR display".to_string();
    projection.status_text = "No active work".to_string();
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260507-0808".to_string()),
        worktree_path: Some(repo.join("work/20260507-0808")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(2538),
        pr_state: Some("OPEN".to_string()),
        pr_url: Some("https://github.com/akiojin/gwt/pull/2538".to_string()),
        pr_created_at: Some("2026-05-07T08:20:00Z".parse().expect("pr created_at")),
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_agents, 0);
    assert_eq!(view.pr_number, Some(2538));
    assert_eq!(view.pr_state.as_deref(), Some("OPEN"));
    assert_eq!(
        view.pr_url.as_deref(),
        Some("https://github.com/akiojin/gwt/pull/2538")
    );
    assert_eq!(
        view.pr_created_at.as_deref(),
        Some("2026-05-07T08:20:00+00:00")
    );
}

#[test]
fn app_runtime_active_work_projection_hides_cleanup_candidate_for_live_agent_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260507-0200".to_string()),
        worktree_path: Some(repo.join("work/20260507-0200")),
        base_branch: Some("origin/main".to_string()),
        pr_number: Some(2525),
        pr_state: Some("merged".to_string()),
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_dirty_branches
        .insert(repo.clone(), HashSet::new());
    runtime
        .work_live_process_branches
        .insert(repo.clone(), HashSet::new());
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id,
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260507-0200".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("work/20260507-0200"),
            agent_project_root: repo.join("work/20260507-0200").display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.cleanup_candidate, None);
}

#[test]
fn app_runtime_active_work_projection_hides_row_cleanup_candidate_for_live_agent_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260615-live-cleanup".to_string()),
        worktree_path: Some(repo.join("work/20260615-live-cleanup")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(3099),
        pr_state: Some("MERGED".to_string()),
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_dirty_branches
        .insert(repo.clone(), HashSet::new());
    runtime
        .work_live_process_branches
        .insert(repo.clone(), HashSet::new());
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id,
            session_id: "session-live-cleanup".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260615-live-cleanup".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("work/20260615-live-cleanup"),
            agent_project_root: repo
                .join("work/20260615-live-cleanup")
                .display()
                .to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/20260615-live-cleanup"))
        .expect("live Workspace row");

    assert!(row.merged_into_base, "merged badge remains visible");
    assert_eq!(
        row.cleanup_candidate, None,
        "live Agent branch must be absent from row-level cleanup candidates"
    );
}

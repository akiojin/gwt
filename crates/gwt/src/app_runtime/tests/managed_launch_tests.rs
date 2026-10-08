use super::*;

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

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::intake");
    session.session_id = "session-intake".to_string();
    session.branch_name = String::new();
    session.worktree_path = intake.clone();
    session.window_id = "tab-1::intake".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::intake".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::intake");

    assert!(!runtime.active_agent_sessions.contains_key("tab-1::intake"));
    assert!(
        !intake.exists(),
        "clean intake worktree is removed when the session ends"
    );
    assert_eq!(
        codex_project_trust_level(&codex_config_path, &codex_project_key),
        None,
        "removing the managed worktree must revoke its Codex project trust"
    );
    let active_work_count = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .map(|view| view.active_works.len())
        .unwrap_or(0);
    assert_eq!(
        active_work_count, 0,
        "an ephemeral intake session emits no Work identity (paused or otherwise)"
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

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::intake");
    session.session_id = "session-intake-dirty".to_string();
    session.branch_name = String::new();
    session.worktree_path = intake.clone();
    session.window_id = "tab-1::intake".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::intake".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::intake");

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

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::intake");
    session.session_id = "session-intake-invalid-codex-config".to_string();
    session.branch_name = String::new();
    session.worktree_path = intake.clone();
    session.window_id = "tab-1::intake".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::intake".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::intake");

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

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::intake");
    session.session_id = "session-intake-docker".to_string();
    session.branch_name = String::new();
    session.worktree_path = intake.clone();
    session.window_id = "tab-1::intake".to_string();
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    runtime
        .active_agent_sessions
        .insert("tab-1::intake".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::intake");

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

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::real");
    session.session_id = "session-real".to_string();
    session.branch_name = "feature/real".to_string();
    session.worktree_path = branch_wt.clone();
    session.window_id = "tab-1::real".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::real".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::real");

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
        let mut agent = workspace_agent_summary_for_test("session-paused", Some("work-paused"));
        agent.window_id = Some("tab-1::agent-paused".to_string());
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
    gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
        .expect("record work event");

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

    runtime.mark_agent_session_stopped("tab-1::agent-paused");

    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load works")
        .expect("works projection");
    let paused = works
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-paused")
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
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-done", Some("work-done"));
        agent.window_id = Some("tab-1::agent-done".to_string());
        agent.branch = Some("work/done".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-done");
    session.session_id = "session-done".to_string();
    session.branch_name = "work/done".to_string();
    session.worktree_path = repo.join("work/done");
    session.window_id = "tab-1::agent-done".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-done".to_string(), session);

    // Stop → Paused row retained on the active surface.
    runtime.mark_agent_session_stopped("tab-1::agent-done");
    let paused_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("paused projection view");
    assert_eq!(paused_view.active_works.len(), 1);
    assert_eq!(paused_view.active_works[0].lifecycle_state, "paused");

    // Close (Done): the Work leaves the active surface.
    let events = runtime.close_work(&runtime.test_context(), "work-session-session-done", "done");
    assert!(
        events.is_empty(),
        "close_work schedules the refreshed projection off the event-loop path"
    );

    let closed_view = wait_for_active_work_projection(&mut runtime);
    assert!(
        closed_view
            .active_works
            .iter()
            .all(|work| work.id != "work-session-session-done"),
        "Done-closed Work must not appear in active_works"
    );

    // The retained work history records the Done terminal close.
    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load work items");
    let item = works
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-done")
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
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-discard", Some("work-discard"));
        agent.window_id = Some("tab-1::agent-discard".to_string());
        agent.branch = Some("work/discard".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-discard");
    session.session_id = "session-discard".to_string();
    session.branch_name = "work/discard".to_string();
    session.worktree_path = repo.join("work/discard");
    session.window_id = "tab-1::agent-discard".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-discard".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::agent-discard");

    let events = runtime.close_work(
        &runtime.test_context(),
        "work-session-session-discard",
        "discarded",
    );
    assert!(events.is_empty());

    let closed_view = wait_for_active_work_projection(&mut runtime);
    assert!(
        closed_view
            .active_works
            .iter()
            .all(|work| work.id != "work-session-session-discard"),
        "Discarded Work must not appear in active_works"
    );

    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load work items");
    let item = works
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-discard")
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
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    // A real worktree directory so we can assert it is NOT removed.
    let worktree_path = repo.join("work/live");
    fs::create_dir_all(&worktree_path).expect("create worktree dir");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-live", Some("work-live"));
        agent.window_id = Some("tab-1::agent-live".to_string());
        agent.branch = Some("work/live".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-live");
    session.session_id = "session-live".to_string();
    session.branch_name = "work/live".to_string();
    session.worktree_path = worktree_path.clone();
    session.window_id = "tab-1::agent-live".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-live".to_string(), session);

    // Agent is live: close must be blocked.
    let events = runtime.close_work(&runtime.test_context(), "work-session-session-live", "done");
    assert!(
        events.is_empty(),
        "a blocked close must not broadcast a projection change"
    );
    assert!(
        worktree_path.exists(),
        "blocked close must never remove the live worktree"
    );

    // No terminal close was recorded; the Work remains live/active.
    let live_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("live projection view");
    let work = live_view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-session-live")
        .expect("live Work still present");
    assert_eq!(work.lifecycle_state, "active");
    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load work items");
    assert!(
        works
            .work_items
            .iter()
            .find(|item| item.id == "work-session-session-live")
            .is_none_or(|item| !item.is_terminal()),
        "a blocked close must not record a terminal event"
    );
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

    // Stop → paused marker persisted to work history.
    runtime.mark_agent_session_stopped("tab-1::agent-resume");
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
    for (session_id, branch) in [("session-live", "work/live"), ("session-stop", "work/stop")] {
        let mut agent = workspace_agent_summary_for_test(session_id, Some(session_id));
        agent.window_id = Some(format!("tab-1::agent-{session_id}"));
        agent.branch = Some(branch.to_string());
        projection.agents.push(agent);
    }
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (session_id, branch) in [("session-live", "work/live"), ("session-stop", "work/stop")] {
        let window_id = format!("tab-1::agent-{session_id}");
        let mut session = sample_active_agent_session("tab-1", &window_id);
        session.session_id = session_id.to_string();
        session.branch_name = branch.to_string();
        session.worktree_path = repo.join(branch);
        session.window_id = window_id.clone();
        runtime.active_agent_sessions.insert(window_id, session);
    }

    // Stop only the second agent; the first remains live.
    runtime.mark_agent_session_stopped("tab-1::agent-session-stop");

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    assert_eq!(view.active_works.len(), 2, "live + paused Work rows");
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-session-live")
        .expect("live Work row");
    assert_eq!(live.lifecycle_state, "active");
    let paused = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-session-stop")
        .expect("paused Work row");
    assert_eq!(paused.lifecycle_state, "paused");
    assert_eq!(paused.active_agents, 0);
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

    super::super::assign_and_merge_workspace_groups(&mut view.active_works, &repo);
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

    super::super::assign_and_merge_workspace_groups(&mut view.active_works, &repo);
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
            event: BackendEvent::WindowCanvasState { .. },
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

    let events = capture_tracing_events(|| {
        let _ = runtime.handle_launch_complete_and_drain(
                window_id,
                Err("failed OPENAI_API_KEY=sk-test --api-key sk-other GWT_HOOK_TOKEN=hook-secret --token plain-token".into()),
            );
    });

    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("launch_complete")
        })
        .expect("redacted launch completion failure log");
    let error = event.fields.get("error").expect("error field");
    assert!(!error.contains("sk-test"));
    assert!(!error.contains("sk-other"));
    assert!(!error.contains("hook-secret"));
    assert!(!error.contains("plain-token"));
    assert!(error.contains("OPENAI_API_KEY=[REDACTED]"));
    assert!(error.contains("--api-key [REDACTED]"));
    assert!(error.contains("GWT_HOOK_TOKEN=[REDACTED]"));
    assert!(error.contains("--token [REDACTED]"));
}

#[test]
fn launch_failure_retains_structured_retry_metadata_in_terminal_status() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");

    use super::super::launch::AgentLaunchError;
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
        let events = runtime.handle_launch_complete_and_drain(window_id.clone(), Err(error));
        let response = events
            .iter()
            .find(|event| {
                matches!(
                    &event.event,
                    BackendEvent::TerminalStatus { id, status: WindowProcessStatus::Error, .. }
                        if id == &window_id
                )
            })
            .expect("caller launch failure response");
        let value = serde_json::to_value(&response.event).expect("serialize launch response");
        assert_eq!(value["error_code"], error_code);
        assert_eq!(value["retryable"], retryable);
    }
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
    let predecessor_incarnation = super::super::next_window_runtime_incarnation();
    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime::new(
            predecessor_incarnation,
            Arc::new(Mutex::new(long_running_test_pane(&window_id))),
        ),
    );
    runtime.stop_window_runtime(&window_id);

    let successor_incarnation = super::super::next_window_runtime_incarnation();
    assert!(
        successor_incarnation > predecessor_incarnation,
        "runtime incarnations must increase monotonically within the process"
    );
    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime::new(
            successor_incarnation,
            Arc::new(Mutex::new(long_running_test_pane(&window_id))),
        ),
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
    let successor_screen = runtime
        .runtimes
        .get(&window_id)
        .expect("successor runtime")
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
    runtime.stop_window_runtime_without_session_projection(&window_id);
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
    runtime.stop_window_runtime_without_session_projection(&window_id);
}

#[test]
fn production_bound_agent_launch_publishes_exact_runtime_and_natural_exit_retains_it() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let session_id = "bound-runtime-natural-exit";
    let identity = install_manual_launch_holder(
        &mut runtime,
        &repo,
        session_id,
        gwt_agent::AgentStatus::Running,
        None,
    );
    let handshake = gwt::cli::execution_state::begin_active_session_launch_handshake(
        &runtime.sessions_dir,
        &identity,
    )
    .expect("begin bound launch handshake")
    .expect("acquire bound launch handshake");
    let runtime_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, session_id);
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
        .save(&runtime_path)
        .expect("save pre-spawn runtime state");
    let window_id = combined_window_id("tab-1", "agent-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let mut completion =
        bound_runtime_launch_completion(&repo, session_id, identity.clone(), command, args);
    completion.11.active_launch_handshake = Some(handshake);
    let events = runtime.handle_launch_complete_and_drain(window_id.clone(), Ok(completion));

    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Error,
            ..
        }
    )));
    let incarnation = runtime
        .runtimes
        .get(&window_id)
        .expect("spawned bound runtime")
        .incarnation;
    let running = gwt_agent::SessionRuntimeState::load(&runtime_path)
        .expect("load exact Running runtime state");
    assert_eq!(running.status, gwt_agent::AgentStatus::Running);
    assert_eq!(running.execution_identity.as_ref(), Some(&identity));
    assert_eq!(running.runtime_incarnation, Some(incarnation));
    assert!(running.child_pid.is_some_and(|pid| pid > 0));
    assert!(running
        .child_started_at
        .is_some_and(|started_at| started_at > 0));
    assert!(
        !gwt_agent::active_launch_handshake_path(&runtime.sessions_dir, session_id).exists(),
        "Running publication must clear the exact child-spawned gate marker"
    );
    wait_for_recorded_event("bound runtime natural exit", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::RuntimeStatus {
                    id,
                    incarnation: event_incarnation,
                    status: WindowProcessStatus::Stopped,
                    ..
                } if id == &window_id && *event_incarnation == incarnation
            )
        })
    });
    runtime.handle_runtime_status_event(
        window_id.clone(),
        incarnation,
        WindowProcessStatus::Stopped,
        Some("Process exited".to_string()),
        true,
    );
    wait_for_recorded_event(
        "bound runtime natural-exit finalizer",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::WindowCloseFinalized {
                        window_id: finalized_window_id,
                        ..
                    } if finalized_window_id == &window_id
                )
            })
        },
    );

    let stopped = gwt_agent::SessionRuntimeState::load(&runtime_path)
        .expect("load naturally stopped runtime state");
    assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(stopped.execution_identity.as_ref(), Some(&identity));
    assert_eq!(stopped.runtime_incarnation, Some(incarnation));
    assert_eq!(stopped.child_pid, running.child_pid);
    assert_eq!(stopped.child_started_at, running.child_started_at);
}

#[test]
fn unbound_agent_pty_publishes_process_identity_for_session_observation() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let geometry = tab
        .workspace
        .window("agent-1")
        .expect("window")
        .geometry
        .clone();
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = gwt_agent::Session::new(&repo, "main", gwt_agent::AgentId::Codex);
    session.project_state_root = Some(repo.clone());
    session.save(&runtime.sessions_dir).expect("Session");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = session.id.clone();
    active.worktree_path = repo.clone();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);
    let runtime_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, &session.id);
    let mut before = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
    before.source_event = Some("session-start".to_string());
    let mut predecessor = session.clone();
    predecessor.repo_hash = Some("previous-repo".to_string());
    predecessor.linked_issue_number = Some(4305);
    predecessor.execution_binding = Some(gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session.id.clone(),
        repo_hash: "previous-repo".to_string(),
        owner_kind: "issue".to_string(),
        owner_number: 4305,
        identity: gwt_agent::ExecutionBindingIdentity {
            generation_id: "previous-generation".to_string(),
            binding_id: "previous-binding".to_string(),
            ledger_head_hash: "previous-head".to_string(),
        },
        capability_generation: 1,
    });
    before.execution_identity = gwt_agent::SessionExecutionIdentity::from_session(&predecessor)
        .expect("previous producing identity");
    before.save(&runtime_path).expect("initial runtime");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec!["/c".to_string(), "pause".to_string()],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-c".to_string(), "read line".to_string()],
        )
    };

    runtime
        .spawn_process_window_with_console_kind(
            &window_id,
            geometry,
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
                resource_policy: None,
            },
            None,
        )
        .expect("spawn unbound agent");

    let observed = gwt_agent::SessionRuntimeState::load(&runtime_path).expect("runtime proof");
    let inventory = gwt::session_inventory::observe_sessions(&repo, &runtime.sessions_dir);
    // Close only this test's PTY even when the following assertions fail.
    runtime.active_agent_sessions.remove(&window_id);
    runtime.stop_window_runtime_without_session_projection(&window_id);
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
    gwt_agent::persist_session_status(
        &runtime.sessions_dir,
        &session.id,
        gwt_agent::AgentStatus::Stopped,
    )
    .expect("persist stop");
    let stopped = gwt_agent::SessionRuntimeState::load(&runtime_path).expect("stopped proof");
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
    let bounds = tab
        .workspace
        .window("legacy-codex")
        .expect("window")
        .geometry
        .clone();
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
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

    runtime.restart_window_events(&codex_id);
    runtime.create_window_events(&runtime.test_context(), WindowPreset::Claude, bounds);

    let inventory = gwt::session_inventory::observe_sessions(&repo, &runtime.sessions_dir);
    let live_windows = runtime.runtimes.keys().cloned().collect::<Vec<_>>();
    for id in &live_windows {
        runtime.stop_window_runtime(id);
    }
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
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session_id = "bound-runtime-replaced";
    let identity = install_manual_launch_holder(
        &mut runtime,
        &repo,
        session_id,
        gwt_agent::AgentStatus::Running,
        None,
    );
    let runtime_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, session_id);
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Idle)
        .save(&runtime_path)
        .expect("save sentinel runtime state");
    let sentinel_bytes = fs::read(&runtime_path).expect("read sentinel runtime state");
    let session_path = runtime.sessions_dir.join(format!("{session_id}.toml"));
    let mut replacement = gwt_agent::Session::load(&session_path).expect("load bound Session");
    replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
    replacement
        .save(&runtime.sessions_dir)
        .expect("save same-id replacement");
    let replacement_bytes = fs::read(&session_path).expect("read replacement Session");
    let window_id = combined_window_id("tab-1", "agent-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "ping -n 31 127.0.0.1 >NUL".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "sleep 30".to_string()],
        )
    };

    let events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Ok(bound_runtime_launch_completion(
            &repo, session_id, identity, command, args,
        )),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Error,
            detail: Some(detail),
            ..
        } if detail.contains("Session identity changed")
    )));
    assert!(!runtime.runtimes.contains_key(&window_id));
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        fs::read(&runtime_path).expect("read retained sentinel runtime state"),
        sentinel_bytes,
    );
    assert_eq!(
        fs::read(&session_path).expect("read retained replacement Session"),
        replacement_bytes,
    );
}

#[test]
fn production_bound_agent_spawn_failure_never_publishes_exact_runtime_proof() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session_id = "bound-runtime-spawn-failure";
    let identity = install_manual_launch_holder(
        &mut runtime,
        &repo,
        session_id,
        gwt_agent::AgentStatus::Running,
        None,
    );
    let runtime_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, session_id);
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
        .save(&runtime_path)
        .expect("save pre-spawn runtime state");
    let window_id = combined_window_id("tab-1", "agent-1");
    super::super::launch::set_bound_pty_gate_program_for_test(PathBuf::from(
        "/definitely/missing/gwt-pty-start-gate",
    ));

    let events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Ok(bound_runtime_launch_completion(
            &repo,
            session_id,
            identity,
            "/definitely/missing/gwt-bound-agent".to_string(),
            Vec::new(),
        )),
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus {
                status: WindowProcessStatus::Error,
                ..
            }
        )),
        "spawn failure events: {events:?}"
    );
    assert!(!runtime.runtimes.contains_key(&window_id));
    let runtime_state = gwt_agent::SessionRuntimeState::load(&runtime_path)
        .expect("load failed spawn runtime state");
    assert!(runtime_state.execution_identity.is_none());
    assert!(runtime_state.runtime_incarnation.is_none());
}

#[test]
fn ordinary_bound_runtime_stop_publishes_terminal_proof_only_after_process_exit() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let session_id = "ordinary-bound-runtime-stop";
    let identity = install_manual_launch_holder(
        &mut runtime,
        &repo,
        session_id,
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    let issuer = install_manual_holder_capability(&mut runtime, &repo, &window_id, &identity);
    let pane = Arc::new(Mutex::new(long_running_test_pane(&window_id)));
    let child_pid = pane
        .lock()
        .expect("pane")
        .pty()
        .process_id()
        .expect("child pid");
    let child_started_at =
        gwt::process::host_process_start_time(child_pid).expect("child process start time");
    let host_started_at =
        gwt::process::host_process_start_time(std::process::id()).expect("Host process start time");
    let incarnation = super::super::next_window_runtime_incarnation();
    runtime
        .runtimes
        .insert(window_id.clone(), WindowRuntime::new(incarnation, pane));
    gwt_agent::persist_session_running_state_if_execution_identity_matches(
        &runtime.sessions_dir,
        &identity,
        incarnation,
        host_started_at,
        child_pid,
        child_started_at,
    )
    .expect("publish exact Running proof");

    runtime.stop_window_runtime(&window_id);

    wait_for_test_pty_stop_settlement(
        &runtime.sessions_dir,
        session_id,
        &identity,
        incarnation,
        child_pid,
        child_started_at,
    );
    assert_eq!(
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("load terminal Session")
            .status,
        gwt_agent::AgentStatus::Stopped
    );
    let proof = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(
        &runtime.sessions_dir,
        session_id,
    ))
    .expect("load terminal runtime proof");
    assert_eq!(proof.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(proof.execution_identity.as_ref(), Some(&identity));
    assert_eq!(proof.runtime_incarnation, Some(incarnation));
    assert_eq!(proof.child_pid, Some(child_pid));
    assert_eq!(proof.child_started_at, Some(child_started_at));
    assert!(
        !gwt_agent::manual_handoff_path(&runtime.sessions_dir, session_id).exists(),
        "ordinary close must release the successor-only durable fence"
    );
    issuer
        .issue_bound(&repo, session_id, identity.execution_binding.clone())
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
    let pty = runtime
        .runtimes
        .get(&window_id)
        .expect("current runtime")
        .pty
        .clone();
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
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
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
    insert_exited_test_pane_runtime(&mut runtime, &window_id, 0);

    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some("Process exited".to_string()),
        true,
    );

    let reloaded = gwt_agent::Session::load(&runtime.sessions_dir.join("session-1.toml"))
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
    insert_exited_test_pane_runtime(&mut runtime, &window_id, 7);

    let _ = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("Process exited with status 1".to_string()),
        true,
    );

    let reloaded = gwt_agent::Session::load(&runtime.sessions_dir.join("session-1.toml"))
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
    if let Some(runtime) = runtime.runtimes.get(&window_id) {
        if let Ok(pane) = runtime.pane.lock() {
            let _ = pane.kill();
        }
    }
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
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
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
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![agent_tab], Some("tab-1"));
    let agent_window_id = combined_window_id("tab-1", "agent-1");
    let shell_window_id = combined_window_id("tab-1", "shell-1");
    insert_test_pane_runtime(&mut runtime, &agent_window_id);
    insert_test_pane_runtime(&mut runtime, &shell_window_id);
    runtime
        .runtimes
        .get(&agent_window_id)
        .expect("agent runtime")
        .pane
        .lock()
        .expect("agent pane")
        .process_bytes(b"unexpected fatal: config parse error\r\n");
    runtime
        .runtimes
        .get(&shell_window_id)
        .expect("shell runtime")
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
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    // The heartbeat itself goes out over the daemon control channel, which a
    // unit test cannot observe. Assert the decision instead: after a hook
    // arrival the runtime must have recorded that this window showed activity.
    runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PostToolUse",
        "session-1",
    ));

    assert!(
        runtime.last_agent_activity_for_test(&window_id).is_some(),
        "a hook arrival must refresh the activity clock for its window"
    );
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

use super::*;

#[test]
fn load_log_entries_from_dir_returns_outcome_with_no_skipped_lines() {
    let dir = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(dir.path());
    write_canonical_log_file(dir.path(), &[PROD_LINE_INFO]);

    let outcome = super::super::load_log_entries_from_dir(dir.path()).expect("read ok");

    assert_eq!(outcome.entries.len(), 1);
    assert_eq!(outcome.entries[0].message, "PTY resize completed");
    assert_eq!(outcome.diagnostics.skipped, 0);
}

#[test]
fn load_log_entries_from_dir_counts_skipped_lines() {
    let dir = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(dir.path());
    write_canonical_log_file(
        dir.path(),
        &[PROD_LINE_INFO, MALFORMED_LINE, PROD_LINE_INFO],
    );

    let outcome = super::super::load_log_entries_from_dir(dir.path()).expect("read ok");

    assert_eq!(outcome.entries.len(), 2);
    assert_eq!(outcome.diagnostics.skipped, 1);
}

#[test]
fn load_log_entries_from_dir_returns_empty_outcome_when_file_missing() {
    let dir = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(dir.path());

    let outcome = super::super::load_log_entries_from_dir(dir.path()).expect("read ok");

    assert!(outcome.entries.is_empty());
    assert_eq!(outcome.diagnostics.skipped, 0);
}

#[test]
fn skipped_lines_warning_is_warn_severity_and_includes_count_and_path() {
    let diagnostics = gwt_core::logging::ReadDiagnostics {
        path: PathBuf::from("/tmp/gwt.log.2026-05-20"),
        skipped: 3,
    };

    let event = super::super::skipped_lines_warning(&diagnostics);

    assert_eq!(event.severity, LogLevel::Warn);
    assert_eq!(event.source, "gwt_core::logging::reader");
    assert!(event.message.contains("Skipped 3 malformed lines"));
    assert!(event.message.contains("/tmp/gwt.log.2026-05-20"));
}

#[test]
fn skipped_lines_warning_singular_for_one_line() {
    let diagnostics = gwt_core::logging::ReadDiagnostics {
        path: PathBuf::from("/tmp/x.log"),
        skipped: 1,
    };

    let event = super::super::skipped_lines_warning(&diagnostics);

    assert!(event.message.contains("Skipped 1 malformed line "));
}

#[test]
fn os_url_open_command_keeps_oauth_query_intact_and_avoids_cmd() {
    // Regression: `cmd /C start "" <url>` truncated OAuth authorize URLs at
    // the first `&`, dropping redirect_uri/scope/state and breaking Slack
    // sign-in. The opener must pass the whole URL as one argument and must
    // not route through cmd.exe (whose shell parsing splits on `&`).
    let url = "https://slack.com/oauth/v2/authorize?client_id=A&redirect_uri=http%3A%2F%2F127.0.0.1%3A8765%2Foauth%2Fcallback&scope=chat%3Awrite%2Cchannels%3Aread&state=xyz";
    let (program, args) = super::super::os_url_open_command(url);

    assert!(
        args.iter().any(|arg| arg == url),
        "the full URL must be passed as a single intact argument, got {args:?}"
    );
    assert_ne!(program, "cmd", "must not open URLs through cmd.exe");
    assert!(
        !args.iter().any(|arg| arg.contains("start")),
        "must not use the cmd `start` builtin which splits on &: {args:?}"
    );
}

/// SPEC-2359 Phase W-15 (FR-380): the Backfill kind reaches the frontend as
/// "backfill" on the wire.
#[test]
fn workspace_work_event_kind_wire_maps_backfill() {
    assert_eq!(
        super::super::workspace_work_event_kind_wire(
            gwt_core::workspace_projection::WorkEventKind::Backfill
        ),
        "backfill"
    );
}

/// SPEC-2359 Phase W-15 (FR-379/FR-382, T-547): a real linked worktree with no
/// Work record is backfilled by `reconcile_workspace_worktrees` and surfaces
/// on the Workspace list as a Paused row. Repeated reconciliation is
/// idempotent (SC-255).
#[test]
fn app_runtime_reconcile_workspace_worktrees_backfills_existing_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
        ["commit", "--allow-empty", "-m", "init"].as_slice(),
    ] {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let worktree = temp.path().join("repo-foo");
    let output = gwt_core::process::hidden_command("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "work/foo",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&repo)
        .output()
        .expect("git worktree add");
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // SPEC-2359 Phase W-15 (FR-382): deliberately NO saved WorkspaceProjection
    // (current.json) and NO live agent session — a fresh home must still
    // surface backfilled records (the list must not depend on live agents or
    // on a previously launched project).
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.reconcile_workspace_worktrees(&repo);
    runtime.reconcile_workspace_worktrees(&repo);

    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    let projection =
        gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("works projection exists");
    let expected_main = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        repo_head_branch(&repo).as_deref(),
        None,
    );
    let expected_foo =
        gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/foo"), None)
            .expect("canonical id");
    assert!(
        projection
            .work_items
            .iter()
            .any(|item| item.id == expected_foo),
        "work/foo worktree must be backfilled: {:?}",
        projection
            .work_items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    );
    let foo_count = projection
        .work_items
        .iter()
        .filter(|item| item.id == expected_foo)
        .count();
    assert_eq!(foo_count, 1, "repeated reconcile must not duplicate items");
    let _ = expected_main;

    assert_eq!(
        load_tracked_work_events(&worktree).len(),
        1,
        "exactly one backfill event after two reconciles"
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == expected_foo)
        .expect("backfilled Workspace row on the surface");
    assert_eq!(row.lifecycle_state, "paused");
    assert_eq!(row.branch.as_deref(), Some("work/foo"));
}

#[test]
fn startup_orphan_intake_prune_dispatch_returns_before_inspection_finishes() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let worker = super::super::startup::spawn_startup_orphan_intake_prune_with(
        vec![repo.clone()],
        move |project_root| {
            started_tx
                .send(project_root.to_path_buf())
                .expect("signal worker start");
            release_rx.recv().expect("release worker");
            0
        },
    )
    .expect("spawn startup recovery worker");

    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker started"),
        repo
    );
    assert!(
        !worker.is_finished(),
        "startup dispatch must return while the safety inspection remains blocked"
    );
    release_tx.send(()).expect("release worker");
    worker.join().expect("startup recovery worker joins");
}

#[test]
fn planned_orphan_intake_path_is_not_queued_for_auto_resume_while_prune_is_blocked() {
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

    let planned_parent = temp.path().join("planned");
    let retained_parent = temp.path().join("retained");
    fs::create_dir_all(&planned_parent).expect("create planned parent");
    fs::create_dir_all(&retained_parent).expect("create retained parent");
    let planned_intake = planned_parent.join(".intake-race");
    let retained_same_basename = retained_parent.join(".intake-race");
    let manager = gwt_git::WorktreeManager::new(&repo);
    manager
        .create_detached("HEAD", &planned_intake)
        .expect("create planned detached intake");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/intake-lookalike",
            retained_same_basename
                .to_str()
                .expect("retained worktree path"),
        ],
    );
    assert_eq!(
        planned_intake.file_name(),
        retained_same_basename.file_name(),
        "the control session must share the basename so exclusion cannot use a name prefix"
    );

    let plan = crate::plan_orphan_intake_worktree_prune(&repo).expect("startup prune plan");
    let planned_paths = plan
        .detached_worktree_paths()
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    assert!(
        planned_paths
            .iter()
            .any(|planned| same_worktree_path(planned, &planned_intake)),
        "the plan may canonicalize a path alias but must retain the same detached worktree"
    );
    assert!(
        !planned_paths
            .iter()
            .any(|planned| same_worktree_path(planned, &retained_same_basename)),
        "a branch-backed worktree with the same basename is not in the detached prune plan"
    );

    let mut persisted = empty_workspace_state();
    for (window_id, session_id) in [
        ("planned-agent", "session-planned-intake"),
        ("retained-agent", "session-retained-intake"),
    ] {
        let mut window =
            sample_window(window_id, WindowPreset::Agent, WindowProcessStatus::Stopped);
        window.agent_id = Some("codex".to_string());
        window.session_id = Some(session_id.to_string());
        persisted.windows.push(window);
    }
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-repo".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    for (session_id, native_session_id, worktree_path, branch) in [
        (
            "session-planned-intake",
            "native-planned-intake",
            planned_intake.as_path(),
            "work/planned-intake",
        ),
        (
            "session-retained-intake",
            "native-retained-intake",
            retained_same_basename.as_path(),
            "work/intake-lookalike",
        ),
    ] {
        let mut session = gwt_agent::Session::new(worktree_path, branch, gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = super::super::startup::spawn_startup_orphan_intake_prune_with(
        vec![(repo, plan)],
        move |_job| {
            started_tx.send(()).expect("signal prune worker start");
            release_rx.recv().expect("release prune worker");
            0
        },
    )
    .expect("spawn blocked prune worker");
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("prune worker reached barrier");

    runtime.queue_startup_auto_resume_sessions(&planned_paths);

    let pending_session_ids = runtime
        .pending_startup_auto_resume_sessions
        .iter()
        .map(|pending| pending.session.id.as_str())
        .collect::<Vec<_>>();

    release_tx.send(()).expect("release prune worker");
    worker.join().expect("prune worker joins");

    assert_eq!(
        pending_session_ids,
        vec!["session-retained-intake"],
        "only the exact detached path in the prune plan must be excluded"
    );
}

/// SPEC-2359 Phase W-16 (FR-402, T-571): a Workspace row whose record has no
/// agents gains them from the machine-local session ledger — sessions whose
/// TOML carries the same repo hash and branch attach to the row with their
/// conversation history, and `session_agent_total` reports the count.
#[test]
fn app_runtime_active_work_projection_attaches_registry_sessions() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
        ["commit", "--allow-empty", "-m", "init"].as_slice(),
    ] {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .expect("run git");
        assert!(output.status.success());
    }
    let worktree = temp.path().join("repo-foo");
    let output = gwt_core::process::hidden_command("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "work/foo",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&repo)
        .output()
        .expect("git worktree add");
    assert!(output.status.success());

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    // Machine-local ledger entry for the branch (no record agents exist).
    let mut session =
        gwt_agent::Session::new(&worktree, "work/foo", gwt_agent::AgentId::ClaudeCode);
    session.agent_session_id = Some("conv-1".to_string());
    session.session_history = vec![gwt_agent::AgentSessionHistoryEntry {
        agent_session_id: "conv-1".to_string(),
        started_at: chrono::Utc::now(),
    }];
    assert!(
        session.repo_hash.is_some(),
        "fixture session must derive the repo hash from its worktree"
    );
    session.save(&runtime.sessions_dir).expect("save session");

    runtime.reconcile_workspace_worktrees(&repo);

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let expected_id =
        gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/foo"), None).unwrap();
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == expected_id)
        .expect("backfilled Workspace row");
    assert_eq!(
        row.agents.len(),
        1,
        "ledger session must attach to the branch Workspace"
    );
    assert_eq!(row.agents[0].session_id, session.id);
    assert_eq!(row.agents[0].display_name, "Claude Code");
    assert_eq!(row.agents[0].sessions.len(), 1);
    assert_eq!(row.agents[0].sessions[0].agent_session_id, "conv-1");
    assert_eq!(
        row.works.len(),
        1,
        "backfilled Workspace has one child Work"
    );
    assert_eq!(
        row.works[0].agents.len(),
        1,
        "ledger session must also attach to the child Work"
    );
    assert_eq!(row.works[0].agents[0].session_id, session.id);
    assert_eq!(row.works[0].agents[0].sessions.len(), 1);
    assert_eq!(
        row.works[0].agents[0].sessions[0].agent_session_id,
        "conv-1"
    );
    assert_eq!(row.session_agent_total, 1);
}

/// SPEC-2359 Phase W-16 (FR-402): the wire cap applies to the row's TOTAL
/// agents (record agents included), not just registry additions — a
/// decomposed legacy row can carry hundreds of record agents and must not
/// flood the workspace payload. The uncapped count rides
/// `session_agent_total`, newest agents win.
#[test]
fn attach_registry_sessions_caps_total_agents_on_the_wire() {
    let agents: Vec<gwt::ActiveWorkAgentView> = (0..12)
        .map(|index| gwt::ActiveWorkAgentView {
            session_id: format!("rec-{index:02}"),
            window_id: None,
            agent_id: format!("custom-agent-{index:02}"),
            display_name: format!("Claude {index:02}"),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: "idle".to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: format!("2026-06-10T12:{index:02}:00Z"),
            sessions: Vec::new(),
        })
        .collect();
    let mut works = vec![gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "work-develop-7ea5aa57".to_string(),
        title: "develop".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("develop".to_string()),
        worktree_path: None,
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents,
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert_eq!(works[0].session_agent_total, 12, "uncapped count reported");
    assert_eq!(
        works[0].agents.len(),
        crate::workspace_session_registry::REGISTRY_SESSION_CAP,
        "wire payload capped"
    );
    assert_eq!(
        works[0].agents[0].session_id, "rec-11",
        "newest agents win the cap"
    );
}

/// User verification 2026-06-17 (follow-up): Workspace detail is a session
/// summary, not a live process inventory. Per agent identity only the latest
/// entry stays; live (active/running/blocked) duplicates are collapsed too.
#[test]
fn attach_registry_sessions_keeps_latest_entry_per_agent_identity() {
    fn agent_with_conv(
        session_id: &str,
        display_name: &str,
        status_category: &str,
        updated_at: &str,
        conversation: &str,
    ) -> gwt::ActiveWorkAgentView {
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: String::new(),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: status_category.to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: updated_at.to_string(),
            sessions: vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: conversation.to_string(),
                started_at: updated_at.to_string(),
                is_active: true,
                resumable: true,
            }],
        }
    }

    let mut works = vec![gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "work-develop-7ea5aa57".to_string(),
        title: "develop".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("develop".to_string()),
        worktree_path: None,
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            agent_with_conv(
                "c1",
                "Claude Code",
                "idle",
                "2026-06-11T10:00:00Z",
                "conv-c1",
            ),
            agent_with_conv(
                "c2",
                "Claude Code",
                "idle",
                "2026-06-12T02:00:00Z",
                "conv-c2",
            ),
            agent_with_conv(
                "c3",
                "Claude Code",
                "idle",
                "2026-06-10T08:00:00Z",
                "conv-c3",
            ),
            agent_with_conv("x1", "Codex", "idle", "2026-06-09T00:00:00Z", "conv-x1"),
            // A live agent of the same identity is still a duplicate in the
            // Workspace session summary.
            agent_with_conv(
                "c-live",
                "Claude Code",
                "active",
                "2026-06-08T00:00:00Z",
                "conv-l",
            ),
        ],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    let agents = &works[0].agents;
    let ids: Vec<&str> = agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect();
    assert!(
        ids.contains(&"c2"),
        "newest Claude Code history entry stays"
    );
    assert!(ids.contains(&"x1"), "the other agent identity stays");
    assert!(
        !ids.contains(&"c-live"),
        "older live duplicate collapses under the latest Claude Code row"
    );
    assert!(!ids.contains(&"c1"), "older duplicates collapse");
    assert!(!ids.contains(&"c3"), "older duplicates collapse");
    assert_eq!(agents.len(), 2);
    assert_eq!(
        works[0].session_agent_total, 5,
        "hidden same-agent candidates stay counted for the '+N more sessions' summary"
    );
}

#[test]
fn workspace_execution_diagnosis_view_preserves_backend_classification() {
    let view = super::super::workspace_execution_diagnosis_view(
        gwt::cli::execution_state::ExecutionDiagnosisSnapshot {
            schema_version: 1,
            ecr_status: gwt::cli::execution_state::ExecutionDiagnosisState::Blocked,
            owner_kind: Some(gwt::cli::execution_state::ExecutionOwnerKind::Spec),
            owner_number: Some(3393),
            blocked_reason: Some("verification evidence is stale".to_string()),
            missing_verification: Some("user confirmation".to_string()),
            generation_id: Some("generation-2".to_string()),
            binding_state: gwt::cli::execution_state::ExecutionBindingState::Stale,
            binding_cause: "current_session_not_authorized".to_string(),
            verification_state: "stale_fingerprint".to_string(),
            trivial_reason: None,
            generated_outputs: vec!["artifacts/report.json".to_string()],
            capability_generation: Some(3),
            continuation: None,
            workspace_update_applicable: Some(false),
            workspace_update_applicability_reason: Some(
                "workspace_update_authority_mismatch".to_string(),
            ),
            obligation_revival: None,
            binding_repair: None,
            repair: None,
            work_event_receipt_generation_id: Some("generation-1".to_string()),
            work_event_receipt_matches_current_generation: Some(false),
            settlement: Some(
                gwt::cli::verification_record::WorkEventSettlementStatus::Blocked(
                    gwt::cli::verification_record::WorkEventSettlementBlocker::MissingUpstream,
                ),
            ),
            settlement_dirty_paths: Vec::new(),
            settlement_severity: "warning".to_string(),
            settlement_obligation_open: true,
            open_obligations: vec!["user_verification".to_string()],
            recovery_probes: Vec::new(),
            available_recoveries: vec!["verify.run".to_string(), "execution.reopen".to_string()],
            recovery_hint: None,
            warnings: vec!["Host status is temporarily unavailable".to_string()],
            launch_route: Some("manual".to_string()),
            permission_decision: None,
            permission_readiness: None,
        },
    );

    assert_eq!(view.ecr_status, "blocked");
    assert_eq!(view.binding_state, "stale");
    assert_eq!(view.verification_state, "stale_fingerprint");
    assert_eq!(view.capability_generation, Some(3));
    assert_eq!(
        view.work_event_receipt_generation_id.as_deref(),
        Some("generation-1")
    );
    assert_eq!(
        view.work_event_receipt_matches_current_generation,
        Some(false)
    );
    assert_eq!(
        view.generated_outputs,
        vec!["artifacts/report.json".to_string()]
    );
    assert_eq!(view.settlement_severity, "warning");
    assert_eq!(
        view.settlement,
        Some(serde_json::json!({"blocked": "missing_upstream"}))
    );
    assert_eq!(
        view.available_recoveries,
        vec!["verify.run", "execution.reopen"]
    );
}

#[test]
fn attach_registry_sessions_preserves_same_identity_agent_per_child_work() {
    let older = workspace_test_agent_with_conversation(
        "older-session",
        "2026-07-12T01:00:00Z",
        "older-conv",
    );
    let newer = workspace_test_agent_with_conversation(
        "newer-session",
        "2026-07-13T01:00:00Z",
        "newer-conv",
    );
    let mut works = vec![workspace_test_work(
        vec![older.clone(), newer.clone()],
        vec![
            workspace_test_child("work-session-older-session", vec![older]),
            workspace_test_child("work-session-newer-session", vec![newer]),
        ],
    )];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert_eq!(
        works[0]
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["newer-session"],
        "the Workspace summary still keeps only the latest agent identity"
    );
    for (work_id, session_id) in [
        ("work-session-older-session", "older-session"),
        ("work-session-newer-session", "newer-session"),
    ] {
        let child = works[0]
            .works
            .iter()
            .find(|child| child.id == work_id)
            .expect("child Work");
        assert_eq!(
            child
                .agents
                .iter()
                .map(|agent| agent.session_id.as_str())
                .collect::<Vec<_>>(),
            vec![session_id],
            "each child Work must preserve its own Agent and Session"
        );
    }
}

#[test]
fn attach_registry_sessions_keeps_usable_agent_only_within_one_child_work() {
    let usable = workspace_test_agent_with_conversation(
        "usable-session",
        "2026-07-12T01:00:00Z",
        "usable-conv",
    );
    let mut empty = workspace_test_agent_with_conversation(
        "empty-session",
        "2026-07-13T01:00:00Z",
        "discarded-conv",
    );
    empty.sessions.clear();
    let mut works = vec![workspace_test_work(
        vec![usable.clone(), empty.clone()],
        vec![workspace_test_child("work-combined", vec![usable, empty])],
    )];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert_eq!(
        works[0].works[0]
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["usable-session"],
        "one child Work renders one canonical Agent identity and prefers its usable conversation"
    );
    assert_eq!(
        works[0]
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["usable-session"],
        "the Workspace summary uses the same conversation-aware identity selection"
    );
}

#[test]
fn attach_registry_sessions_preserves_punctuation_distinct_custom_agent_identities() {
    let mut hyphenated = workspace_test_agent_with_conversation(
        "custom-hyphen-session",
        "2026-07-13T01:00:00Z",
        "custom-hyphen-conversation",
    );
    hyphenated.agent_id = "my-agent".to_string();
    hyphenated.display_name = "my-agent".to_string();
    let mut compact = workspace_test_agent_with_conversation(
        "custom-compact-session",
        "2026-07-12T01:00:00Z",
        "custom-compact-conversation",
    );
    compact.agent_id = "myagent".to_string();
    compact.display_name = "myagent".to_string();
    let mut works = vec![workspace_test_work(
        vec![hyphenated.clone(), compact.clone()],
        vec![workspace_test_child(
            "work-custom-agents",
            vec![hyphenated, compact],
        )],
    )];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert_eq!(
        works[0].works[0]
            .agents
            .iter()
            .map(|agent| agent.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["my-agent", "myagent"],
        "unknown custom agent commands retain exact trimmed spelling; punctuation is identity-significant",
    );
    assert_eq!(
        works[0]
            .agents
            .iter()
            .map(|agent| agent.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["my-agent", "myagent"],
    );
}

#[test]
fn attach_registry_sessions_collapses_same_conversation_across_child_works() {
    let older = workspace_test_agent_with_conversation(
        "older-session",
        "2026-07-12T01:00:00Z",
        "shared-conv",
    );
    let newer = workspace_test_agent_with_conversation(
        "newer-session",
        "2026-07-13T01:00:00Z",
        "shared-conv",
    );
    let mut works = vec![workspace_test_work(
        vec![older.clone(), newer.clone()],
        vec![
            workspace_test_child("work-session-older-session", vec![older]),
            workspace_test_child("work-session-newer-session", vec![newer]),
        ],
    )];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert!(
        works[0].works[0].agents.is_empty(),
        "the older sibling must not retain a duplicate conversation"
    );
    assert_eq!(
        works[0].works[1].agents[0].session_id, "newer-session",
        "the newest sibling owns the shared conversation"
    );
    assert_eq!(works[0].session_agent_total, 1);
}

#[test]
fn attach_registry_sessions_caps_agents_across_all_child_works() {
    let agents = (0..12)
        .map(|index| {
            workspace_test_agent_with_conversation(
                &format!("session-{index:02}"),
                &format!("2026-07-12T{index:02}:00:00Z"),
                &format!("conv-{index:02}"),
            )
        })
        .collect::<Vec<_>>();
    let child_works = agents
        .iter()
        .cloned()
        .map(|agent| {
            workspace_test_child(&format!("work-session-{}", agent.session_id), vec![agent])
        })
        .collect();
    let mut works = vec![workspace_test_work(agents, child_works)];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert_eq!(
        works[0]
            .works
            .iter()
            .map(|child| child.agents.len())
            .sum::<usize>(),
        crate::workspace_session_registry::REGISTRY_SESSION_CAP,
        "the wire cap applies to the union of child Work agents"
    );
    assert!(
        works[0]
            .works
            .iter()
            .take(4)
            .all(|child| child.agents.is_empty()),
        "the oldest sessions fall outside the cap"
    );
}

#[test]
fn attach_registry_sessions_assigns_shared_session_to_one_canonical_child_work() {
    let agent = workspace_test_agent_with_conversation(
        "shared-session",
        "2026-07-13T01:00:00Z",
        "shared-conv",
    );
    let mut child_works = (0..11)
        .map(|index| workspace_test_child(&format!("legacy-work-{index:02}"), vec![agent.clone()]))
        .collect::<Vec<_>>();
    child_works.insert(
        5,
        workspace_test_child("work-session-shared-session", vec![agent.clone()]),
    );
    let mut works = vec![workspace_test_work(vec![agent], child_works)];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert_eq!(
        works[0]
            .works
            .iter()
            .map(|child| child.agents.len())
            .sum::<usize>(),
        1,
        "one payload Agent must not be duplicated across child Works"
    );
    assert_eq!(
        works[0]
            .works
            .iter()
            .find(|child| child.id == "work-session-shared-session")
            .expect("canonical child Work")
            .agents[0]
            .session_id,
        "shared-session",
        "the canonical session-derived child owns the shared Agent"
    );
    assert_eq!(works[0].session_agent_total, 1);
}

#[test]
fn attach_registry_sessions_assigns_shared_session_to_latest_legacy_child_work() {
    let agent = workspace_test_agent_with_conversation(
        "shared-session",
        "2026-07-13T01:00:00Z",
        "shared-conv",
    );
    let mut older = workspace_test_child("legacy-work-older", vec![agent.clone()]);
    older.updated_at = "2026-07-12T01:00:00Z".to_string();
    let mut newer = workspace_test_child("legacy-work-newer", vec![agent.clone()]);
    newer.updated_at = "2026-07-13T01:00:00Z".to_string();
    let mut works = vec![workspace_test_work(vec![agent], vec![older, newer])];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    assert!(works[0].works[0].agents.is_empty());
    assert_eq!(
        works[0].works[1].agents[0].session_id, "shared-session",
        "the latest compatible legacy child owns the shared Agent"
    );
}

#[test]
fn attach_registry_sessions_recomputes_agent_counters_after_identity_collapse() {
    fn agent_view(
        session_id: &str,
        display_name: &str,
        status_category: &str,
        updated_at: &str,
    ) -> gwt::ActiveWorkAgentView {
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: String::new(),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: status_category.to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: updated_at.to_string(),
            sessions: vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: format!("conv-{session_id}"),
                started_at: updated_at.to_string(),
                is_active: true,
                resumable: true,
            }],
        }
    }

    let mut works = vec![gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "work-develop-7ea5aa57".to_string(),
        title: "develop".to_string(),
        status_category: "active".to_string(),
        status_text: "Active".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 99,
        blocked_agents: 99,
        branch: Some("develop".to_string()),
        worktree_path: None,
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            agent_view(
                "claude-old",
                "Claude Code",
                "active",
                "2026-06-10T00:00:00Z",
            ),
            agent_view(
                "claude-new",
                "Claude Code",
                "running",
                "2026-06-11T00:00:00Z",
            ),
            agent_view("codex-blocked", "Codex", "blocked", "2026-06-09T00:00:00Z"),
        ],
        works: Vec::new(),
        lifecycle_state: "active".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    let ids: Vec<&str> = works[0]
        .agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["claude-new", "codex-blocked"],
        "latest visible agent per identity determines counters"
    );
    assert_eq!(works[0].active_agents, 1);
    assert_eq!(works[0].blocked_agents, 1);
}

/// User verification 2026-06-12 (follow-up): a record agent whose ledger TOML
/// is gone and that recorded no identity and no conversation renders as a
/// dead "Agent / No session yet" group whose Resume cannot work. Such ghosts
/// are dropped from the view; identifiable or conversation-bearing agents stay.
#[test]
fn attach_registry_sessions_drops_ghost_agents_without_identity_or_sessions() {
    fn bare_agent(session_id: &str, display_name: &str) -> gwt::ActiveWorkAgentView {
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: String::new(),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: "idle".to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: "2026-06-12T02:00:00Z".to_string(),
            sessions: Vec::new(),
        }
    }

    let mut works = vec![gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "work-work-x-12345678".to_string(),
        title: "work/x".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/x".to_string()),
        worktree_path: None,
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            bare_agent("gwt-ghost", ""),
            bare_agent("gwt-named", "Claude Code"),
        ],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    let agents = &works[0].agents;
    assert_eq!(
        agents.len(),
        1,
        "the identity-less, session-less ghost is dropped"
    );
    assert_eq!(agents[0].session_id, "gwt-named");
    assert_eq!(works[0].session_agent_total, 1);
}

/// User verification 2026-06-12: Work records written without agent metadata
/// (older record paths) rendered as an anonymous "Agent" group. The view
/// borrows display_name / agent_id from the ledger TOML keyed by the gwt
/// session id, so the group is named whenever the ledger still knows it.
#[test]
fn agent_view_borrows_identity_from_ledger_when_record_has_none() {
    let mut session = gwt_agent::Session::new(
        std::path::PathBuf::from("/tmp/none"),
        "work/foo",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "gwt-session-anon".to_string();
    session.display_name = "Claude Code".to_string();
    let mut index = std::collections::HashMap::new();
    index.insert("gwt-session-anon", &session);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: "gwt-session-anon".to_string(),
        agent_id: None,
        display_name: None,
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };
    let view = super::super::workspace_work_agent_view_from_ref(
        &agent_ref,
        &index,
        scanned_without_branches(),
    );
    assert_eq!(view.display_name.as_deref(), Some("Claude Code"));
    assert!(view.agent_id.is_some(), "agent_id borrowed from the ledger");
}

/// User verification 2026-06-12: a Resume creates a NEW gwt session for the
/// SAME agent conversation, so the row showed two Work groups with the same
/// conversation id ("Agent" 15m ago + "Claude Code" 1d ago). Agents whose
/// latest conversation matches collapse into one row — newest wins, and a
/// missing display_name is borrowed from the duplicate.
#[test]
fn attach_registry_sessions_dedupes_agents_sharing_a_conversation() {
    fn agent_view(
        session_id: &str,
        display_name: &str,
        updated_at: &str,
        conversation: &str,
    ) -> gwt::ActiveWorkAgentView {
        let agent_id = if display_name == "Codex" {
            "codex"
        } else {
            "claude"
        };
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: agent_id.to_string(),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: "idle".to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: updated_at.to_string(),
            sessions: vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: conversation.to_string(),
                started_at: updated_at.to_string(),
                is_active: true,
                resumable: true,
            }],
        }
    }

    let mut works = vec![gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "work-work-x-12345678".to_string(),
        title: "work/x".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/x".to_string()),
        worktree_path: None,
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            // Resume-created record agent: no display_name recorded yet.
            agent_view("gwt-new", "", "2026-06-12T02:05:00Z", "conv-shared"),
            // The original session for the same conversation, a day older.
            agent_view(
                "gwt-old",
                "Claude Code",
                "2026-06-10T04:15:00Z",
                "conv-shared",
            ),
            // A different conversation must survive untouched.
            agent_view("gwt-other", "Codex", "2026-06-11T00:00:00Z", "conv-other"),
        ],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    let agents = &works[0].agents;
    assert_eq!(agents.len(), 2, "shared conversation collapses to one row");
    let kept = agents
        .iter()
        .find(|agent| agent.sessions[0].agent_session_id == "conv-shared")
        .expect("shared conversation row");
    assert_eq!(kept.session_id, "gwt-new", "newest gwt session wins");
    assert_eq!(
        kept.display_name, "Claude Code",
        "missing display_name is borrowed from the duplicate"
    );
    assert!(agents
        .iter()
        .any(|agent| agent.sessions[0].agent_session_id == "conv-other"));
    assert_eq!(
        works[0].session_agent_total, 2,
        "the collapsed duplicate is not counted as a hidden extra session"
    );
}

/// User verification 2026-06-19: a legacy branchless Work record can carry
/// agent refs from multiple branches. Once such refs reach a branch-backed row,
/// the row must drop sessions whose ledger branch/worktree belongs to another
/// Workspace so the same Codex conversation is not shown under two Workspaces.
#[test]
fn attach_registry_sessions_filters_agents_from_other_workspace_rows() {
    fn agent_view(
        session_id: &str,
        display_name: &str,
        conversation: &str,
    ) -> gwt::ActiveWorkAgentView {
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: display_name.to_ascii_lowercase().replace(' ', "-"),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: "idle".to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: "2026-06-19T13:49:00Z".to_string(),
            sessions: vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: conversation.to_string(),
                started_at: "2026-06-19T13:49:00Z".to_string(),
                is_active: true,
                resumable: true,
            }],
        }
    }

    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let issue_worktree = temp.path().join("unity-cli/work/issue-206");
    let other_worktree = temp.path().join("unity-cli/work/20260616-1102");
    fs::create_dir_all(&issue_worktree).expect("issue worktree");
    fs::create_dir_all(&other_worktree).expect("other worktree");

    let mut issue_session = gwt_agent::Session::new(
        &issue_worktree,
        "work/issue-206",
        gwt_agent::AgentId::ClaudeCode,
    );
    issue_session.id = "78992500-1502-4ab2-8e67-04f79803e013".to_string();
    issue_session.agent_session_id = Some("33939943-240d-461f-bf90-e7b5497e4ee8".to_string());
    issue_session.display_name = "Claude Code".to_string();
    let mut other_session = gwt_agent::Session::new(
        &other_worktree,
        "work/20260616-1102",
        gwt_agent::AgentId::Codex,
    );
    other_session.id = "5b907840-31ee-48d5-a7e3-277c93fda63b".to_string();
    other_session.agent_session_id = Some("019ed018-c208-7183-bb6e-b08ba2ef4981".to_string());
    other_session.display_name = "Codex".to_string();
    let mut session_index = std::collections::HashMap::new();
    session_index.insert(issue_session.id.as_str(), &issue_session);
    session_index.insert(other_session.id.as_str(), &other_session);

    let mut works = vec![gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "work-work-issue-206-a0668517".to_string(),
        title: "contribution docs PR".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: Some("Issue #206".to_string()),
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/issue-206".to_string()),
        worktree_path: Some(issue_worktree.display().to_string()),
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            agent_view(
                &issue_session.id,
                "Claude Code",
                "33939943-240d-461f-bf90-e7b5497e4ee8",
            ),
            agent_view(
                &other_session.id,
                "Codex",
                "019ed018-c208-7183-bb6e-b08ba2ef4981",
            ),
        ],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: "2026-06-19T13:49:00Z".to_string(),
    }];

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &session_index,
        scanned_without_branches(),
    );

    let agents = &works[0].agents;
    assert_eq!(agents.len(), 1, "only sessions owned by this row stay");
    assert_eq!(agents[0].display_name, "Claude Code");
    assert_eq!(
        agents[0].sessions[0].agent_session_id,
        "33939943-240d-461f-bf90-e7b5497e4ee8"
    );
    assert!(
        agents
            .iter()
            .flat_map(|agent| agent.sessions.iter())
            .all(|session| session.agent_session_id != "019ed018-c208-7183-bb6e-b08ba2ef4981"),
        "Codex conversation from work/20260616-1102 must not appear on work/issue-206"
    );
}

/// SPEC-2359 Phase W-16 (FR-402 follow-up, user verification 2026-06-10): on
/// this machine none of the ledger TOMLs carry `session_history` (the field
/// is newer than the sessions), but almost all carry `agent_session_id` (the
/// latest conversation). The view must synthesize that latest conversation as
/// a single Session row instead of rendering "No session yet" everywhere.
#[test]
fn agent_view_synthesizes_latest_conversation_when_history_is_empty() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let mut session =
        gwt_agent::Session::new(&worktree, "work/foo", gwt_agent::AgentId::ClaudeCode);
    session.id = "gwt-session-1".to_string();
    session.agent_session_id = Some("conv-latest".to_string());
    session.session_history = Vec::new();
    let mut index = std::collections::HashMap::new();
    index.insert(session.id.as_str(), &session);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: "gwt-session-1".to_string(),
        agent_id: Some("claude".to_string()),
        display_name: Some("Claude Code".to_string()),
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };

    let view = super::super::workspace_work_agent_view_from_ref(
        &agent_ref,
        &index,
        scanned_without_branches(),
    );

    assert_eq!(
        view.sessions.len(),
        1,
        "latest conversation must be synthesized when history is empty"
    );
    assert_eq!(view.sessions[0].agent_session_id, "conv-latest");
    assert!(view.sessions[0].is_active);
    assert!(view.sessions[0].resumable);
}

#[test]
fn agent_view_marks_session_history_only_when_worktree_and_branch_are_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let mut session = gwt_agent::Session::new(
        temp.path().join("deleted-worktree"),
        "feature/deleted-session-row",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "gwt-session-missing-branch".to_string();
    session.agent_session_id = Some("conv-latest".to_string());
    let mut index = std::collections::HashMap::new();
    index.insert(session.id.as_str(), &session);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: session.id.clone(),
        agent_id: Some("claude".to_string()),
        display_name: Some("Claude Code".to_string()),
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };

    let view = super::super::workspace_work_agent_view_from_ref(
        &agent_ref,
        &index,
        scanned_without_branches(),
    );

    assert_eq!(view.sessions.len(), 1);
    assert!(
        !view.sessions[0].resumable,
        "missing worktree plus missing local/origin branch is history-only"
    );
}

#[test]
fn agent_view_keeps_session_resumable_when_missing_worktree_branch_exists() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "feature/session-row-resume";
    run_git(&repo, &["branch", branch]);
    let mut session = gwt_agent::Session::new(
        temp.path().join("deleted-worktree"),
        branch,
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "gwt-session-existing-branch".to_string();
    session.agent_session_id = Some("conv-latest".to_string());
    let mut index = std::collections::HashMap::new();
    index.insert(session.id.as_str(), &session);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: session.id.clone(),
        agent_id: Some("claude".to_string()),
        display_name: Some("Claude Code".to_string()),
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };

    let known_branch_refs = super::super::resume_branch_refs_snapshot(&repo);
    let view = super::super::workspace_work_agent_view_from_ref(
        &agent_ref,
        &index,
        super::super::ResumeBranchIndex::scanned(Some(&known_branch_refs)),
    );

    assert_eq!(view.sessions.len(), 1);
    assert!(
        view.sessions[0].resumable,
        "available branch lets exact Session Resume re-materialize the worktree"
    );
}

/// Issue #3611: the snapshot is keyed by short ref name, so both the local
/// (`work/x`) and the remote-tracking (`origin/work/x`) form must satisfy the
/// same check the retired `git show-ref refs/heads` + `refs/remotes/origin`
/// pair performed.
#[test]
fn resume_branch_index_matches_local_and_origin_ref_names() {
    let refs = HashSet::from([
        "work/local-only".to_string(),
        "origin/work/remote-only".to_string(),
    ]);
    let index = super::super::ResumeBranchIndex::scanned(Some(&refs));

    assert!(index.branch_exists("work/local-only"));
    assert!(index.branch_exists("work/remote-only"));
    assert!(index.branch_exists("origin/work/remote-only"));
    assert!(index.branch_exists("refs/remotes/origin/work/remote-only"));
    assert!(!index.branch_exists("work/deleted"));
    assert!(!index.branch_exists("   "));
}

/// Issue #3611: before the first background scan there is no snapshot to
/// consult. Answering "not resumable" would hide a working Resume control on
/// every project open, so an unscanned project stays optimistic — the Launch
/// Wizard re-verifies branch existence before it materializes anything.
#[test]
fn resume_branch_index_stays_optimistic_until_the_project_is_scanned() {
    let unscanned = super::super::ResumeBranchIndex::scanned(None);
    assert!(unscanned.branch_exists("work/not-yet-scanned"));
    assert!(
        !unscanned.branch_exists(""),
        "an empty branch name is never materializable"
    );

    let no_branches = HashSet::new();
    let scanned_empty = super::super::ResumeBranchIndex::scanned(Some(&no_branches));
    assert!(
        !scanned_empty.branch_exists("work/not-yet-scanned"),
        "a completed scan that found nothing is a real negative verdict"
    );
}

/// Issue #3611: a live worktree is decided by a filesystem stat alone — no
/// snapshot needed and no process, even when the branch is unknown.
#[test]
fn resume_branch_index_accepts_existing_worktree_without_branch_evidence() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("live-worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let session = gwt_agent::Session::new(&worktree, "work/unknown", gwt_agent::AgentId::Codex);

    assert!(
        super::super::ResumeBranchIndex::scanned(Some(&HashSet::new()))
            .session_exact_resume_materializable(&session)
    );
}

/// SPEC-2359 Phase W-16 (FR-403): the Workspace list is ordered by last
/// update, newest first — rows with fresher records or fresher ledger
/// sessions float to the top, stale backfill rows sink.
#[test]
fn active_works_are_sorted_by_latest_update_descending() {
    let row = |id: &str, branch: &str, updated_at: &str| gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: id.to_string(),
        title: branch.to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some(branch.to_string()),
        worktree_path: None,
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: Vec::new(),
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: updated_at.to_string(),
    };
    let mut works = vec![
        row("work-old", "work/old", "2026-05-01T00:00:00Z"),
        row("work-new", "work/new", "2026-06-10T09:00:00Z"),
        row("work-mid", "work/mid", "2026-06-01T00:00:00Z"),
    ];
    // The mid row carries a fresher ledger session than its record stamp, so
    // it must outrank the 06-10 09:00 row.
    works[2].agents.push(gwt::ActiveWorkAgentView {
        session_id: "sess-fresh".to_string(),
        window_id: None,
        agent_id: "claude".to_string(),
        display_name: "Claude".to_string(),
        affiliation_status: "assigned".to_string(),
        workspace_id: None,
        status_category: "idle".to_string(),
        current_focus: None,
        title_summary: None,
        branch: None,
        worktree_path: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        updated_at: "2026-06-10T12:00:00Z".to_string(),
        sessions: Vec::new(),
    });

    super::super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        scanned_without_branches(),
    );

    let order: Vec<&str> = works.iter().map(|work| work.id.as_str()).collect();
    assert_eq!(
        order,
        vec!["work-mid", "work-new", "work-old"],
        "rows sort by max(record updated_at, agents updated_at) descending"
    );
}

/// SPEC-2359 W-15 (FR-386): rows flagged merged ("safe to delete") via the
/// background scan cache (canonical branch match) or the recorded PR state.
#[test]
fn mark_merged_active_works_flags_cache_and_pr_state() {
    let row = |branch: Option<&str>, pr_state: Option<&str>| gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "w".to_string(),
        title: "t".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: branch.map(str::to_string),
        worktree_path: None,
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: pr_state.map(str::to_string),
        board_refs: Vec::new(),
        agents: Vec::new(),
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    };
    let mut works = vec![
        row(Some("origin/work/merged"), None),
        row(Some("work/open"), None),
        row(None, Some("MERGED")),
    ];
    let merged: HashMap<String, chrono::DateTime<chrono::Utc>> =
        [("work/merged".to_string(), chrono::Utc::now())]
            .into_iter()
            .collect();

    super::super::mark_merged_active_works(&mut works, Some(&merged), None);

    assert!(
        works[0].merged_into_base,
        "cache match (origin/ normalized)"
    );
    assert!(
        !works[1].merged_into_base,
        "unmerged branch stays unflagged"
    );
    assert!(works[2].merged_into_base, "PR state merged flags the row");
}

#[test]
fn dirty_worktree_pr_state_merged_does_not_flag_or_cleanup() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "repo\n").expect("write readme");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "init"]);
    run_git(&repo, &["checkout", "-q", "-b", "work/dirty"]);
    fs::write(repo.join("local-change.txt"), "current edits\n").expect("write dirty file");

    let mut works = vec![gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "w-dirty".to_string(),
        title: "Dirty work".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/dirty".to_string()),
        worktree_path: Some(repo.display().to_string()),
        managed_hook_health: None,
        pr_number: Some(123),
        pr_url: None,
        pr_state: Some("MERGED".to_string()),
        board_refs: Vec::new(),
        agents: Vec::new(),
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: "2026-06-10T12:00:00Z".to_string(),
    }];

    let mut missing_cache = works.clone();
    super::super::mark_merged_active_works(&mut missing_cache, None, None);
    super::super::mark_workspace_cleanup_candidates(&mut missing_cache, None, None, &[], None);
    assert!(
        !missing_cache[0].merged_into_base,
        "missing dirty cache fails closed for PR-derived merged state"
    );
    assert_eq!(
        missing_cache[0].cleanup_candidate, None,
        "missing dirty cache fails closed for cleanup"
    );

    let dirty_branches = HashSet::from(["work/dirty".to_string()]);
    super::super::mark_merged_active_works(&mut works, None, Some(&dirty_branches));
    super::super::mark_workspace_cleanup_candidates(
        &mut works,
        None,
        Some(&dirty_branches),
        &[],
        Some(&HashSet::new()),
    );

    assert!(
        !works[0].merged_into_base,
        "dirty current worktree must not inherit an old merged PR badge"
    );
    assert_eq!(
        works[0].cleanup_candidate, None,
        "dirty current worktree must not become cleanup-ready from old PR state"
    );

    let mut clean_works = works.clone();
    let clean_branches = HashSet::new();
    super::super::mark_merged_active_works(&mut clean_works, None, Some(&clean_branches));
    super::super::mark_workspace_cleanup_candidates(
        &mut clean_works,
        None,
        Some(&clean_branches),
        &[],
        Some(&HashSet::new()),
    );
    assert!(
        clean_works[0].merged_into_base,
        "clean cache evidence preserves the merged PR badge"
    );
    assert!(
        clean_works[0].cleanup_candidate.is_some(),
        "clean cache evidence preserves merged PR cleanup eligibility"
    );
}

/// SPEC-2359 W-15 (FR-386): apply_work_merge_status stores the scan result
/// and the next projection build flags the matching rows.
#[test]
fn apply_work_merge_status_caches_and_flags_rows() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let now = chrono::Utc::now();
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-merged-row",
            now,
        );
        event.title = Some("merged work".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/merged".to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let merged: HashMap<String, chrono::DateTime<chrono::Utc>> =
        [("work/merged".to_string(), chrono::Utc::now())]
            .into_iter()
            .collect();
    let _ = runtime.apply_work_merge_status(
        &repo,
        merged,
        HashMap::new(),
        HashSet::new(),
        HashSet::new(),
        None,
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == "work-merged-row")
        .expect("row");
    assert!(row.merged_into_base, "cached merge scan flags the row");
}

#[cfg(unix)]
#[test]
fn spawn_work_merge_status_scan_skips_dirty_worktree_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "seed\n").expect("seed");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "seed"]);
    run_git(&repo, &["branch", "work/dirty"]);
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/develop", "develop"],
    );
    fs::write(repo.join("local-change.txt"), "uncommitted\n").expect("dirty file");
    let _child = KillOnDrop(
        gwt_core::process::hidden_command("sh")
            .arg("-c")
            .arg("sleep 30")
            .current_dir(&repo)
            .spawn()
            .expect("spawn live cwd process"),
    );

    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-dirty-row",
            chrono::Utc::now(),
        );
        event.title = Some("dirty work".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/dirty".to_string()),
                worktree_path: Some(repo.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.spawn_work_merge_status_scan(repo.clone());

    wait_for_recorded_event("dirty work merge status", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::WorkMergeStatus {
                    project_root,
                    ..
                } if project_root == &repo
            )
        })
    });

    let snapshot = events.lock().expect("event log").clone();
    let (_, merged_branches, cleanup_ready_branches, dirty_branches, live_process_branches) =
        snapshot
            .iter()
            .find_map(|event| match recorded_project_payload(event) {
                UserEvent::WorkMergeStatus {
                    project_root,
                    merged_branches,
                    cleanup_ready_branches,
                    dirty_branches,
                    live_process_branches,
                    ..
                } if project_root == &repo => Some((
                    project_root,
                    merged_branches,
                    cleanup_ready_branches,
                    dirty_branches,
                    live_process_branches,
                )),
                _ => None,
            })
            .expect("work merge status event");

    assert!(
        merged_branches.is_empty(),
        "dirty worktree branch must not render as merged: {merged_branches:?}"
    );
    assert!(
        cleanup_ready_branches.is_empty(),
        "dirty worktree branch must not become cleanup-ready: {cleanup_ready_branches:?}"
    );
    assert_eq!(
        dirty_branches,
        &HashSet::from(["work/dirty".to_string()]),
        "background scan publishes the dirty branch verdict"
    );
    assert_eq!(
        live_process_branches,
        &HashSet::from(["work/dirty".to_string()]),
        "background scan publishes live-process protection with the same completion"
    );
}

/// Issue #4009: gwt rewrites `.codex/hooks.json` on every materialization and
/// appends to its own `.gwt/` namespace on every Work event, so a naive
/// "any status entry means dirty" verdict marked practically every worktree
/// dirty and left `CLEAN UP READY` reporting 0. Only changes outside gwt's own
/// namespaces count as work the user could lose.
#[test]
fn spawn_work_merge_status_scan_treats_gwt_runtime_writes_as_clean() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(repo.join(".codex")).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "seed\n").expect("seed");
    fs::write(repo.join(".codex/hooks.json"), "{\n  \"hooks\": {}\n}\n").expect("seed hooks");
    run_git(&repo, &["add", "README.md", ".codex/hooks.json"]);
    run_git(&repo, &["commit", "-qm", "seed"]);
    run_git(&repo, &["branch", "work/gwt-writes"]);
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/develop", "develop"],
    );
    // The materialization rewrite every launch performs: same managed shape,
    // different bytes, so git reports a tracked modification.
    fs::write(repo.join(".codex/hooks.json"), "{ \"hooks\": {} }\n").expect("rewrite hooks");

    // Recording the Work event is what writes `.gwt/work/`, exactly as the
    // runtime does in a real worktree.
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-gwt-writes-row",
            chrono::Utc::now(),
        );
        event.title = Some("gwt writes only".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/gwt-writes".to_string()),
                worktree_path: Some(repo.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record work");

    // Guard the premise: the naive verdict this replaces saw a dirty worktree
    // here, which is exactly why `CLEAN UP READY` reported 0.
    let status = gwt_git::diff::get_status(&repo).expect("status");
    assert!(
        !status.is_empty(),
        "fixture must leave gwt's own writes uncommitted"
    );

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.spawn_work_merge_status_scan(repo.clone());

    wait_for_recorded_event("gwt-write work merge status", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::WorkMergeStatus {
                    project_root,
                    ..
                } if project_root == &repo
            )
        })
    });

    let snapshot = events.lock().expect("event log").clone();
    let (cleanup_ready_branches, dirty_branches) = snapshot
        .iter()
        .find_map(|event| match recorded_project_payload(event) {
            UserEvent::WorkMergeStatus {
                project_root,
                cleanup_ready_branches,
                dirty_branches,
                ..
            } if project_root == &repo => Some((cleanup_ready_branches, dirty_branches)),
            _ => None,
        })
        .expect("work merge status event");

    assert!(
        dirty_branches.is_empty(),
        "gwt's own runtime writes must not make a branch dirty: {dirty_branches:?}"
    );
    assert!(
        cleanup_ready_branches.contains_key("work/gwt-writes"),
        "a change-free branch whose only diff is gwt's own writes stays cleanup-ready: \
         {cleanup_ready_branches:?}"
    );
    // The worker must warm the persistent cache, not a private per-scan copy.
    let tips = gwt_git::refs::branch_tip_snapshot(&repo).expect("tips");
    let caches = runtime.work_merge_status_cache.borrow();
    let mut cache = caches
        .get(&repo)
        .expect("project merge cache")
        .lock()
        .unwrap();
    let before = gwt_core::process::thread_git_spawn_count();
    assert!(cache
        .base_target(&repo, "work/gwt-writes", &tips)
        .unwrap()
        .is_some());
    assert_eq!(gwt_core::process::thread_git_spawn_count() - before, 0);
    drop(cache);
    drop(caches);
    runtime.invalidate_project_caches(&repo);
    assert!(!runtime.work_merge_status_cache.borrow().contains_key(&repo));
}

#[test]
fn spawn_work_merge_status_scan_preserves_historical_merged_pr_cleanup_path() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let historical_worktree = temp.path().join("historical-worktree");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "seed\n").expect("seed");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "seed"]);
    run_git(&repo, &["branch", "work/historical-merged"]);
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            historical_worktree.to_str().expect("utf-8 worktree"),
            "work/historical-merged",
        ],
    );
    fs::write(historical_worktree.join("feature.txt"), "historical\n").expect("historical feature");
    run_git(&historical_worktree, &["add", "feature.txt"]);
    run_git(
        &historical_worktree,
        &["commit", "-qm", "feat: historical work"],
    );
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/develop", "develop"],
    );

    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Pr,
            "work-historical-merged-row",
            chrono::Utc::now(),
        );
        event.title = Some("Historical merged work".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/historical-merged".to_string()),
                worktree_path: Some(historical_worktree.clone()),
                pr_number: Some(3385),
                pr_url: None,
                pr_state: Some("MERGED".to_string()),
            },
        );
        event
    })
    .expect("record historical merged work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.spawn_work_merge_status_scan(repo.clone());

    wait_for_recorded_event("historical merged work status", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::WorkMergeStatus {
                    project_root,
                    ..
                } if project_root == &repo
            )
        })
    });
    let event = events
        .lock()
        .expect("event log")
        .iter()
        .find_map(|event| match recorded_project_payload(event) {
            UserEvent::WorkMergeStatus {
                project_root,
                merged_branches,
                cleanup_ready_branches,
                dirty_branches,
                live_process_branches,
                known_branch_refs,
            } if project_root == &repo => Some((
                merged_branches.clone(),
                cleanup_ready_branches.clone(),
                dirty_branches.clone(),
                live_process_branches.clone(),
                known_branch_refs.clone(),
            )),
            _ => None,
        })
        .expect("historical work merge status");
    let _ = runtime.apply_work_merge_status(&repo, event.0, event.1, event.2, event.3, event.4);

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == "work-historical-merged-row")
        .expect("historical merged row");

    assert!(
        row.cleanup_candidate.is_some() || row.cleanup_blocked_reason.is_some(),
        "a recorded merged PR with a local historical worktree must retain a Clean Up path \
         or an explicit safety blocker: {row:?}"
    );
}

#[test]
fn work_branch_dirty_scan_failure_is_fail_closed() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let missing_worktree = temp.path().join("missing-worktree");
    let target = super::super::WorkBranchScanTarget {
        branch: "work/missing".to_string(),
        worktree_paths: vec![missing_worktree],
        has_merged_pr: true,
    };

    assert!(
        super::super::work_branch_has_dirty_worktree(&target),
        "an unreadable worktree must block merged and cleanup classification"
    );
}

#[test]
fn work_merge_scan_checks_dirty_state_only_for_actionable_branches() {
    let readiness = gwt_git::branch::CleanupReadinessTarget {
        target: gwt_git::branch::MergeTargetRef::new(
            gwt_git::branch::MergeTarget::Develop,
            "origin/develop",
        ),
        reason: gwt_git::branch::CleanupReadinessReason::Merged,
    };

    assert!(
        !super::super::work_merge_scan_needs_dirty_check(None, false),
        "an unready historical branch without a merged PR cannot consume a dirty verdict"
    );
    assert!(
        super::super::work_merge_scan_needs_dirty_check(Some(&readiness), false),
        "cleanup readiness must remain guarded by a current dirty verdict"
    );
    assert!(
        super::super::work_merge_scan_needs_dirty_check(None, true),
        "a recorded merged PR must remain guarded by a current dirty verdict"
    );
}

#[test]
fn work_branch_scan_targets_preserve_merged_pr_dirty_guard() {
    let now = chrono::Utc::now();
    let mut projection = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Pr,
        "work-merged-pr",
        now,
    );
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("origin/work/merged-pr".to_string()),
            worktree_path: Some(PathBuf::from("/tmp/work-merged-pr")),
            pr_number: Some(42),
            pr_url: None,
            pr_state: Some("MERGED".to_string()),
        },
    );
    let _ = projection.apply_event(event);

    let targets = super::super::work_branch_scan_targets(&projection);

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].branch, "work/merged-pr");
    assert!(
        targets[0].has_merged_pr,
        "case-insensitive merged PR metadata must request a dirty-worktree guard"
    );
}

#[test]
fn spawn_work_merge_status_scan_clears_stale_cache_when_no_targets_remain() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            "work-terminal-row",
            chrono::Utc::now(),
        );
        event.title = Some("terminal work".to_string());
        event.status_category = Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/merged".to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record terminal work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.spawn_work_merge_status_scan(repo.clone());

    wait_for_recorded_event("empty work merge status", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::WorkMergeStatus {
                    project_root,
                    merged_branches,
                    cleanup_ready_branches,
                    dirty_branches,
                    ..
                } if project_root == &repo
                    && merged_branches.is_empty()
                    && cleanup_ready_branches.is_empty()
                    && dirty_branches.is_empty()
            )
        })
    });
}

#[test]
fn apply_work_merge_status_caches_no_changes_cleanup_readiness() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let worktree = repo.join("work/no-changes");
    fs::create_dir_all(&worktree).expect("create worktree");
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-no-changes-row",
            chrono::Utc::now(),
        );
        event.title = Some("no changes work".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/no-changes".to_string()),
                worktree_path: Some(worktree.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let cleanup_ready: HashMap<String, String> =
        [("work/no-changes".to_string(), "no_changes".to_string())]
            .into_iter()
            .collect();
    let _ = runtime.apply_work_merge_status(
        &repo,
        HashMap::new(),
        cleanup_ready,
        HashSet::new(),
        HashSet::new(),
        None,
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == "work-no-changes-row")
        .expect("row");
    let candidate = row
        .cleanup_candidate
        .as_ref()
        .expect("no-changes readiness produces cleanup candidate");

    assert_eq!(candidate.reason, "no_changes");
    assert!(!row.merged_into_base, "no-changes is not a merged badge");

    let _ = runtime.apply_work_merge_status(
        &repo,
        HashMap::new(),
        HashMap::new(),
        HashSet::new(),
        HashSet::new(),
        None,
    );
    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view after cache clear");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == "work-no-changes-row")
        .expect("row after cache clear");
    assert_eq!(
        row.cleanup_candidate, None,
        "empty readiness result clears stale no-changes cleanup candidate"
    );
}

// SPEC-2359 W-17 (FR-396): when a client's queue dropped streamed output,
// the repair path re-sends a fresh full snapshot — client-scoped, and only
// for panes that still have a live runtime.
#[test]
fn client_pane_snapshot_repair_replies_with_snapshots_for_known_panes_only() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    insert_test_pane_runtime(&mut runtime, &shell_id);
    runtime
        .runtimes
        .get(&shell_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane lock")
        .process_bytes(b"hello-repair");

    let events = runtime.client_pane_snapshot_repair_events(
        "client-9",
        &[shell_id.clone(), "tab-1::missing-pane".to_string()],
    );

    assert_eq!(events.len(), 1, "unknown panes produce no repair events");
    assert!(
        matches!(&events[0].target, DispatchTarget::Client(id) if id == "client-9"),
        "repair snapshot is scoped to the requesting client"
    );
    match &events[0].event {
        BackendEvent::TerminalSnapshot { id, data_base64 } => {
            assert_eq!(id, &shell_id);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_base64)
                .expect("snapshot base64");
            assert!(
                String::from_utf8_lossy(&bytes).contains("hello-repair"),
                "snapshot carries the pane's current screen content"
            );
        }
        other => panic!("expected TerminalSnapshot, got {other:?}"),
    }
}

// Issue #4095 AC-1 / AC-2: a snapshot serialized while the PTY reader is ahead
// of the event loop already contains chunks whose `terminal_output` has not
// been dispatched yet. Replaying those chunks after the snapshot re-applies
// relative cursor moves on a screen that already moved — the "✻ Fro" fragment
// rows and overlapping lines from the field screenshot. Both the
// queue-pressure repair snapshot (client-1) and a scrollback re-sync on
// reconnect (client-2) must leave the client screen identical to the pane's.
#[test]
fn pane_snapshot_never_replays_spinner_redraw_chunks_it_already_contains() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        project,
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let window_id = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| combined_window_id("tab-1", &window.id))
        .next()
        .expect("agent window");
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    insert_test_pane_runtime(&mut runtime, &window_id);
    let (incarnation, pane) = {
        let window_runtime = runtime.runtimes.get(&window_id).expect("runtime");
        (window_runtime.incarnation, Arc::clone(&window_runtime.pane))
    };

    let repair_queue = ClientQueue::default();
    let resync_queue = ClientQueue::default();
    let mut resync_connected = false;
    let mut undispatched: Vec<(Vec<u8>, u64)> = Vec::new();
    for (index, chunk) in spinner_redraw_pty_chunks().into_iter().enumerate() {
        // Reader thread: parse under the pane lock and note the position.
        let seq = {
            let mut pane = pane.lock().expect("pane lock");
            pane.process_bytes(&chunk);
            pane.output_seq()
        };
        undispatched.push((chunk, seq));
        // The event loop lags behind the reader for chunks 8..=19.
        let reader_ahead = (8..=19).contains(&index);
        if index == 19 {
            for event in runtime
                .client_pane_snapshot_repair_events("client-1", std::slice::from_ref(&window_id))
            {
                repair_queue.enqueue(&prepare_outbound_event(&event));
            }
            for event in runtime.frontend_sync_events("client-2") {
                resync_queue.enqueue(&prepare_outbound_event(&event));
            }
            resync_connected = true;
        }
        if !reader_ahead || index == 19 {
            for (data, seq) in undispatched.drain(..) {
                for event in
                    runtime.handle_runtime_output_event(window_id.clone(), incarnation, data, seq)
                {
                    let prepared = prepare_outbound_event(&event);
                    repair_queue.enqueue(&prepared);
                    if resync_connected {
                        resync_queue.enqueue(&prepared);
                    }
                }
            }
        }
    }

    let (expected_rows, expected_cursor) = {
        let pane = pane.lock().expect("pane lock");
        (pane.screen().contents(), pane.screen().cursor_position())
    };
    for (client, queue) in [
        ("client-1 repair", &repair_queue),
        ("client-2 resync", &resync_queue),
    ] {
        let replayed = replay_client_terminal(queue);
        assert_eq!(
            replayed.screen().contents(),
            expected_rows,
            "{client}: client screen must equal the pane screen after the snapshot"
        );
        assert_eq!(
            replayed.screen().cursor_position(),
            expected_cursor,
            "{client}: client cursor must equal the pane cursor after the snapshot"
        );
    }
}

// SPEC-2359 W-17 (FR-398, Issue #3034): a second spawn for the same Work
// while the first launch is still materializing (window registered, agent
// session not yet live) must focus the pending window, not spawn a duplicate.
#[test]
fn app_runtime_spawn_agent_window_dedupes_inflight_launch_for_same_work() {
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
    let build_config = || {
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/20260610-inflight")
            .build()
    };

    runtime
        .spawn_agent_window("tab-1", build_config(), canvas_bounds(), None)
        .expect("first spawn");
    runtime
        .spawn_agent_window("tab-1", build_config(), canvas_bounds(), None)
        .expect("second spawn");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_windows = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "in-flight re-click must not spawn a duplicate agent window"
    );
}

// SPEC-2359 W-17 (FR-398): a successful Resume replies a client-scoped
// `workspace_resume_agent_started` ack so pending UI settles deterministically.
#[test]
fn resume_workspace_agent_replies_started_ack_to_requesting_client() {
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
    let session = gwt_agent::Session::new(&repo, "feature/resume-ack", gwt_agent::AgentId::Codex);
    session.save(&sessions_dir).expect("save session");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.resume_workspace_agent_events(
        &runtime.test_context(),
        "client-7",
        "resume-operation-7".to_string(),
        session.id.clone(),
        None,
        canvas_bounds(),
    );

    let ack = events
        .iter()
        .find(|event| {
            matches!(
                &event.event,
                BackendEvent::WorkspaceResumeAgentStarted { session_id, .. }
                    if session_id == &session.id
            )
        })
        .expect("started ack present");
    assert!(
        matches!(&ack.target, DispatchTarget::Client(id) if id == "client-7"),
        "ack is scoped to the requesting client"
    );
    match &ack.event {
        BackendEvent::WorkspaceResumeAgentStarted {
            operation_id,
            branch,
            ..
        } => {
            assert_eq!(operation_id, "resume-operation-7");
            assert_eq!(branch.as_deref(), Some("feature/resume-ack"));
        }
        other => panic!("expected started ack, got {other:?}"),
    }
}

/// Close-latency root fix (2026-06-12): stopping an agent window records the
/// Paused Work marker (FR-350) on a background thread — the works.json
/// load+save must not run on the UI event loop (it reaches megabytes and the
/// synchronous write made × clicks stall for seconds). The record itself must
/// still land; the test polls for it.
#[test]
fn stop_window_runtime_records_paused_work_off_the_event_loop() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-paused-offloop".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/paused-offloop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let started = Instant::now();
    runtime.stop_window_runtime(&window_id);
    let stop_call = started.elapsed();
    // The stop call itself returns promptly (no synchronous multi-MB IO).
    assert!(
        stop_call < Duration::from_secs(2),
        "stop_window_runtime blocked for {stop_call:?}"
    );

    // The Paused Work record still lands (background write).
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut recorded = false;
    while Instant::now() < deadline {
        let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
            .unwrap_or_else(|_| gwt_core::workspace_projection::WorkItemsProjection {
                updated_at: chrono::Utc::now(),
                work_items: Vec::new(),
            });
        recorded = works.work_items.iter().any(|item| {
            item.id == "work-session-session-paused-offloop"
                && item.status_category
                    == gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
        });
        if recorded {
            break;
        }
        // test-hygiene: allow-short-duration bounded polling of worker-persisted Work state; no completion event is exposed by this fixture
        thread::sleep(Duration::from_millis(50));
    }
    assert!(recorded, "Paused Work record must land in works.json");
}

/// SPEC-2359 W-16 (FR-387): the tab-change ingest trigger is throttled to
/// once per 30s per project; bootstrap / project-open callers bypass it.
#[test]
fn work_events_ingest_attempt_is_throttled_per_project() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let root = temp.path().join("repo");

    assert!(runtime.note_work_events_ingest_attempt(&root, false));
    assert!(
        !runtime.note_work_events_ingest_attempt(&root, false),
        "second attempt within 30s is throttled"
    );
    assert!(
        runtime.note_work_events_ingest_attempt(&root, true),
        "force bypasses the throttle"
    );
    let other = temp.path().join("other");
    assert!(
        runtime.note_work_events_ingest_attempt(&other, false),
        "throttle is per project root"
    );
}

/// Issue #3604: the rail's PR-title lookup is a single
/// `gh pr list --state all --limit 999` — up to ten GraphQL requests. It rode
/// the 30s ingest throttle, so a GUI in ordinary use re-enumerated every PR in
/// the repository twice a minute. It now has its own, far longer window.
#[test]
fn work_pr_titles_scan_has_its_own_long_window_and_is_per_project() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let root = temp.path().join("repo");

    assert!(runtime.note_work_pr_titles_scan_attempt(&root));
    assert!(
        !runtime.note_work_pr_titles_scan_attempt(&root),
        "a second PR-title enumeration inside the window is suppressed"
    );

    let other = temp.path().join("other");
    assert!(
        runtime.note_work_pr_titles_scan_attempt(&other),
        "the window is per project root"
    );

    assert!(
        super::super::WORK_PR_TITLES_SCAN_WINDOW >= Duration::from_secs(300),
        "the window must be an order of magnitude wider than the 30s ingest throttle"
    );
}

/// Issue #3604: bounded staleness must not become unbounded staleness. Opening
/// a project is the moment a stale rail summary would be noticed, so the forced
/// ingest reopens the PR-title window.
#[test]
fn reopening_the_pr_titles_window_allows_an_immediate_refresh() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let root = temp.path().join("repo");

    assert!(runtime.note_work_pr_titles_scan_attempt(&root));
    assert!(!runtime.note_work_pr_titles_scan_attempt(&root));

    runtime.reopen_work_pr_titles_window(&root);

    assert!(
        runtime.note_work_pr_titles_scan_attempt(&root),
        "reopening the window must allow the next enumeration through"
    );
}

#[test]
fn terminal_preview_preserves_three_screen_rows_for_live_and_reconnect() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let id = combined_window_id("tab-1", "shell-1");
    insert_test_pane_runtime(&mut runtime, &id);
    runtime.runtimes[&id]
        .pane
        .lock()
        .unwrap()
        .process_bytes(b"old\r\n  indented\r\n\r\nlast\r\n");
    let live = runtime.handle_runtime_output(id.clone(), b"last".to_vec());
    let sync = runtime.frontend_project_sync_events("client-preview", &runtime.test_context());
    for events in [live, sync] {
        let preview = events
            .iter()
            .map(|event| serde_json::to_value(&event.event).unwrap())
            .find(|event| event["kind"] == "terminal_preview")
            .expect("preview event");
        assert_eq!(preview["id"], id);
        assert_eq!(preview["text"], "  indented\n\nlast");
    }
}

#[test]
fn terminal_preview_remote_reconnect_retains_only_received_values() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let id = combined_window_id("tab-1", "agent-1");
    let unknown = runtime.handle_daemon_runtime_output(id.clone(), b"bytes".to_vec(), None);
    assert!(!unknown
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalPreview { .. })));
    let live = runtime.handle_daemon_runtime_output(
        id.clone(),
        b"bytes".to_vec(),
        Some("  remote\n\nlast".into()),
    );
    let sync = runtime.frontend_project_sync_events("client-preview", &runtime.test_context());
    for events in [live, sync] {
        assert!(events.iter().any(|event| matches!(&event.event, BackendEvent::TerminalPreview { id: pane, text } if pane == &id && text == "  remote\n\nlast")));
    }
    let cleared =
        runtime.handle_daemon_runtime_output(id.clone(), b"clear".to_vec(), Some(String::new()));
    assert!(cleared.iter().any(|event| matches!(&event.event, BackendEvent::TerminalPreview { text, .. } if text.is_empty())));
    runtime.remove_window_state_tracking(&id);
    assert!(!runtime.remote_terminal_previews.contains_key(&id));
}

#[cfg(unix)]
#[test]
fn issue_monitor_delivery_claim_publishes_current_canvas_before_daemon_claim() {
    use std::{io::BufRead, os::unix::net::UnixListener};

    use gwt_core::daemon::{
        persist_endpoint, ClientFrame, DaemonEndpoint, DaemonFrame, IpcHandshakeRequest,
        IpcHandshakeResponse, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    };

    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let now = chrono::Utc::now().to_rfc3339();
    let window_id = "tab-1::agent-1";
    let host_pid = std::process::id();
    let host_started_at = gwt::process::host_process_start_time(host_pid).expect("host start time");
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        max_active: 1,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[42], "operator", &now);
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: 42,
        title: "Current canvas adoption".to_string(),
        labels: Vec::new(),
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(42, "claim-42", "host/session", "effect-42", &now));
    assert!(monitor.record_monitor_runtime_windows(
        host_pid,
        host_started_at,
        BTreeMap::from([(42, 1)]),
        BTreeMap::from([(window_id.to_string(), 42)]),
        &now,
    ));
    assert_eq!(
        monitor.active_count(),
        2,
        "the daemon has no canvas binding yet"
    );
    assert!(!monitor.has_capacity_for_monitor_spawn(42, false));
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed unassigned delivery");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut feedback = issue_monitor_feedback(42);
    feedback.issue_monitor_project_root = Some(repo.clone());
    runtime
        .pending_launch_feedback_contexts
        .insert(window_id.to_string(), feedback);
    assert_eq!(
        runtime.window_status(window_id),
        Some(WindowProcessStatus::Starting)
    );

    let scope = RuntimeScope::from_project_root(&repo, RuntimeTarget::Host).expect("runtime scope");
    let socket_path = temp.path().join("claim.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind fixture daemon");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let endpoint = DaemonEndpoint::new(
        scope.clone(),
        host_pid,
        socket_path.display().to_string(),
        "claim-token".to_string(),
        "test-daemon".to_string(),
    );
    persist_endpoint(
        &scope.endpoint_path(&gwt_core::paths::gwt_home()),
        &endpoint,
    )
    .expect("persist fixture endpoint");
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let server = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture daemon runtime");
        let _runtime_guard = runtime.enter();
        // Readiness notifications avoid spending the production IPC budget on fixture polling.
        let listener =
            tokio::net::UnixListener::from_std(listener).expect("register fixture daemon listener");
        ready_tx.send(()).expect("fixture daemon is ready");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut received = Vec::new();
        loop {
            let (stream, _) = runtime
                .block_on(async { tokio::time::timeout_at(deadline, listener.accept()).await })
                .expect("fixture publish arrives before hang guard")
                .expect("accept fixture publish");
            let mut stream = stream.into_std().expect("fixture stream");
            stream
                .set_nonblocking(false)
                .expect("blocking fixture stream");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("bound fixture reads");
            let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read handshake");
            let request: IpcHandshakeRequest =
                serde_json::from_str(line.trim_end()).expect("parse handshake");
            assert_eq!(request.scope, scope);
            writeln!(
                stream,
                "{}",
                serde_json::to_string(&IpcHandshakeResponse {
                    protocol_version: DAEMON_PROTOCOL_VERSION,
                    daemon_version: "test-daemon".to_string(),
                    accepted: true,
                    rejection_reason: None,
                })
                .expect("serialize handshake")
            )
            .expect("write handshake");
            line.clear();
            reader.read_line(&mut line).expect("read publish");
            let ClientFrame::Publish { channel, payload } =
                serde_json::from_str(line.trim_end()).expect("parse publish")
            else {
                panic!("expected control publish");
            };
            assert_eq!(
                channel,
                gwt::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
            );
            assert_eq!(payload["source_pid"].as_u64(), Some(u64::from(host_pid)));
            let control = &payload["payload"];
            let claim = control.get("claim_launch_delivery");
            let accepted = if let Some(claim) = claim {
                received.push("claim");
                monitor.claim_launch_delivery(
                    claim["issue_number"].as_u64().expect("issue number"),
                    claim["delivery_id"].as_str().expect("delivery identity"),
                    claim["materializer_id"]
                        .as_str()
                        .expect("materializer identity"),
                    claim["materializer_pid"]
                        .as_u64()
                        .expect("materializer pid") as u32,
                    claim["materializer_window_id"]
                        .as_str()
                        .expect("pane identity"),
                    gwt::process::is_host_process_alive,
                )
            } else {
                received.push("snapshot");
                let snapshot: gwt::IssueMonitorWindowSnapshot =
                    serde_json::from_value(control["window_snapshot"].clone())
                        .expect("current canvas snapshot");
                let tabs = serde_json::from_value(control["window_snapshot_project_tabs"].clone())
                    .expect("canvas tab scope");
                assert_eq!(snapshot.windows[0].window_id, window_id);
                assert!(snapshot.windows[0].monitor_owned);
                assert_eq!(
                    tabs,
                    std::collections::BTreeSet::from(["tab-1".to_string()])
                );
                monitor.record_window_snapshot_from_host(snapshot, host_pid, host_started_at, tabs);
                true
            };
            gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs())
                .expect("commit daemon prefs");
            writeln!(
                stream,
                "{}",
                serde_json::to_string(&DaemonFrame::Ack).expect("serialize daemon ack")
            )
            .expect("write daemon ack");
            if claim.is_some() {
                return (received, accepted);
            }
        }
    });

    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("fixture daemon starts before the claim");
    let accepted =
        runtime.claim_issue_monitor_launch_delivery(&repo, 42, "launch:effect-42", window_id);
    let server_result = server.join();
    let accepted = accepted.expect("daemon claim outcome");
    let (received, daemon_accepted) = server_result.expect("fixture daemon joins");
    assert_eq!(
        received,
        ["snapshot", "claim"],
        "the same fresh canvas must precede the claim"
    );
    assert!(
        daemon_accepted,
        "the daemon must adopt the exact existing pane at max_active=1"
    );
    assert!(
        accepted,
        "readback must confirm the exact materializer tuple"
    );
    assert_eq!(runtime.tabs[0].workspace.persisted().windows.len(), 1);
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("reload daemon claim");
    let delivery = &prefs.pending_launch_deliveries[0];
    assert_eq!(
        delivery.materializer_id.as_deref(),
        Some(runtime.issue_monitor_materializer_id.as_str())
    );
    assert_eq!(delivery.materializer_pid, Some(host_pid));
    assert_eq!(delivery.materializer_window_id.as_deref(), Some(window_id));
}

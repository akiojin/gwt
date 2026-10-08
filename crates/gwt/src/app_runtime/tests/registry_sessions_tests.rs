use super::*;

/// SPEC-2359 W-16 (FR-387): the ingest worker completes reconcile before the
/// tao continuation, which only commits prepared branch state and schedules a
/// projection refresh when persisted Work state changed.
#[test]
fn handle_work_events_ingested_commits_prepared_state_and_refreshes_only_on_change() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    // Seed one Work record so the projection broadcast has content.
    let mut seed = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-session-ingest-seed",
        chrono::Utc::now(),
    );
    seed.status_category = Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
    seed.title = Some("seed".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&repo, seed)
        .expect("seed work record");

    let unchanged = runtime.handle_work_events_ingested(
        repo.clone(),
        false,
        Some(std::collections::HashSet::from([
            "work/issue-3777".to_string()
        ])),
    );
    assert!(
        unchanged.is_empty(),
        "no-op ingest must not rebroadcast the projection"
    );
    assert_eq!(
        runtime.local_worktree_branches.borrow().get(&repo).cloned(),
        Some(std::collections::HashSet::from([
            "work/issue-3777".to_string()
        ])),
        "the tao continuation commits only the already-prepared branch set",
    );

    let changed = runtime.handle_work_events_ingested(repo, true, None);
    assert!(changed.is_empty());
    assert_eq!(
        tasks.lock().expect("queued tasks").len(),
        1,
        "changed ingest schedules exactly one background projection prepare",
    );
}

/// Issue #4378 AC-1: the startup ingest hands back the worktree inventory the
/// bootstrap already listed, so the reconcile reads it instead of running
/// `git worktree list` again. The project root is not a repository, so a
/// listing of its own would fail and record no local branches.
#[test]
fn handle_work_events_ingested_reconciles_from_the_startup_inventory() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("not-a-repo");
    fs::create_dir_all(&repo).expect("project root");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let inventory = Arc::new(vec![gwt::worktree_inventory::WorktreeEntry {
        id: "shared".to_string(),
        kind: gwt::worktree_inventory::WorktreeEntryKind::Workspace,
        path: temp.path().join("worktrees").join("shared"),
        label: "work/shared".to_string(),
        branch: Some("work/shared".to_string()),
        is_active: false,
    }]);

    // Issue #3777 AC-3: the reconcile itself runs on the ingest worker, so the
    // inventory is consumed there rather than handed back to the tao callback.
    runtime.reconcile_workspace_worktrees_from(&repo, &inventory);

    assert!(
        runtime
            .local_worktree_branches
            .borrow()
            .get(&repo)
            .is_some_and(|branches| branches.contains("work/shared")),
        "the reconcile must use the inventory the startup ingest carried back"
    );
}

#[test]
fn inactive_project_completion_refreshes_projection_cache_before_tab_change() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    fs::create_dir_all(&repo_a).expect("repo-a dir");
    fs::create_dir_all(&repo_b).expect("repo-b dir");
    let tabs = vec![
        sample_project_tab("tab-a", "Repo A", repo_a, ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-b", "Repo B", repo_b.clone(), ProjectKind::NonRepo, &[]),
    ];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-a"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let branch = "work/inactive-cache";
    let mut seed = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-inactive-cache",
        chrono::Utc::now(),
    );
    seed.status_category = Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Idle);
    seed.title = Some(branch.to_string());
    seed.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some(branch.to_string()),
            worktree_path: Some(repo_b.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo_b, seed)
        .expect("seed inactive project work");

    let initial = runtime
        .build_active_work_projection_for_tab_for_test("tab-b", &runtime.tabs[1])
        .expect("initial inactive projection");
    assert_eq!(initial.active_works[0].work_summary, None);

    let events = runtime.apply_work_pr_titles(
        &repo_b,
        HashMap::from([(
            branch.to_string(),
            "Fresh inactive project purpose".to_string(),
        )]),
    );
    assert!(
        events.is_empty(),
        "an inactive project cache refresh must not broadcast into the active tab"
    );
    tasks.lock().expect("queued tasks").remove(0)();
    let completion = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("inactive projection completion");
    let UserEvent::ActiveWorkProjectionPrepared(completion) =
        into_recorded_project_payload(completion)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };
    let dispatch = runtime
        .handle_active_work_projection_prepared(*completion)
        .prepared_dispatch
        .expect("non-selected project completion must reach its own clients");
    assert_eq!(dispatch.context, runtime.project_context("tab-b").unwrap());

    assert!(runtime
        .active_work_projection_broadcast_on_tab_change("tab-b")
        .is_none());
    let dispatch = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("cached tab-change dispatch");
    let UserEvent::PreparedActiveWorkDispatch { payload, .. } = dispatch else {
        panic!("expected PreparedActiveWorkDispatch");
    };
    let payload: serde_json::Value = serde_json::from_str(&payload).expect("prepared payload");
    assert_eq!(
        payload["projection"]["active_works"][0]["work_summary"].as_str(),
        Some("Fresh inactive project purpose"),
        "tab change must use the target project's completion-refreshed cache",
    );
}

/// SPEC-2359 W16-2 (FR-389 / SC-259): two Works on the same canonical branch
/// (any spelling) merge into ONE Workspace row — newest representative,
/// agents concatenated, counts summed — while branchless legacy rows keep
/// their own identity.
#[test]
fn assign_and_merge_workspace_groups_unifies_same_branch_rows() {
    fn row(
        id: &str,
        branch: Option<&str>,
        updated_at: &str,
        agents: usize,
        lifecycle: &str,
    ) -> gwt::ActiveWorkItemView {
        gwt::ActiveWorkItemView {
            linked_issue_numbers: Vec::new(),
            id: id.to_string(),
            title: id.to_string(),
            status_category: "idle".to_string(),
            status_text: "Paused".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: agents,
            blocked_agents: 0,
            branch: branch.map(str::to_string),
            worktree_path: None,
            managed_hook_health: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            board_refs: Vec::new(),
            agents: Vec::new(),
            works: Vec::new(),
            lifecycle_state: lifecycle.to_string(),
            closed_at: None,
            session_agent_total: 1,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            updated_at: updated_at.to_string(),
        }
    }

    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    let mut branch_backed = row(
        "work-work-x-aaaa",
        Some("work/x"),
        "2026-06-11T10:00:00Z",
        0,
        "paused",
    );
    branch_backed.pr_number = Some(3327);
    branch_backed.pr_url = Some("https://github.com/akiojin/gwt/pull/3327".to_string());
    branch_backed.pr_state = Some("MERGED".to_string());
    let mut works = vec![
        branch_backed,
        row(
            "work-session-bbbb",
            Some("origin/work/x"),
            "2026-06-12T10:00:00Z",
            2,
            "active",
        ),
        row(
            "workspace-1748822400000",
            None,
            "2026-06-10T10:00:00Z",
            0,
            "paused",
        ),
    ];

    works[0].linked_issue_numbers = vec![3885];
    works[1].linked_issue_numbers = vec![3885, 4556];
    super::super::assign_and_merge_workspace_groups(&mut works, &root);

    assert_eq!(
        works.len(),
        2,
        "same-branch rows merge; legacy row survives"
    );
    let group = works
        .iter()
        .find(|work| {
            work.branch.as_deref() == Some("origin/work/x")
                || work.branch.as_deref() == Some("work/x")
        })
        .expect("grouped row");
    assert_eq!(group.linked_issue_numbers, vec![3885, 4556]);
    assert_eq!(
        group.id, "work-session-bbbb",
        "newest row is the representative"
    );
    assert_eq!(group.active_agents, 2, "agent counts sum");
    assert_eq!(group.session_agent_total, 2, "session totals sum");
    assert_eq!(group.pr_number, Some(3327));
    assert_eq!(
        group.pr_url.as_deref(),
        Some("https://github.com/akiojin/gwt/pull/3327")
    );
    assert_eq!(
        group.pr_state.as_deref(),
        Some("MERGED"),
        "a newer session row must not erase branch-backed PR metadata"
    );
    assert!(group.workspace_key.is_some());
    assert_eq!(
        group.works.len(),
        2,
        "Workspace grouping must preserve both child Works"
    );
    assert_eq!(
        group
            .works
            .iter()
            .map(|work| work.id.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["work-session-bbbb", "work-work-x-aaaa"]),
        "each child Work keeps its stable identity"
    );
    let paused = group
        .works
        .iter()
        .find(|work| work.id == "work-work-x-aaaa")
        .expect("paused child Work");
    assert_eq!(paused.lifecycle_state, "paused");
    assert!(paused.manual_close_allowed);
    assert_eq!(paused.close_blocked_reason, None);
    let active = group
        .works
        .iter()
        .find(|work| work.id == "work-session-bbbb")
        .expect("active child Work");
    assert!(!active.manual_close_allowed);
    assert_eq!(active.close_blocked_reason.as_deref(), Some("live_agent"));
    let legacy = works
        .iter()
        .find(|work| work.id == "workspace-1748822400000")
        .expect("legacy row");
    assert_eq!(
        legacy.workspace_key.as_deref(),
        Some("workspace-1748822400000")
    );

    let mut older_pr = row(
        "work-pr-precedence",
        Some("work/pr-precedence"),
        "2026-06-11T10:00:00Z",
        0,
        "paused",
    );
    older_pr.pr_number = Some(100);
    older_pr.pr_url = Some("https://example.test/pull/100".to_string());
    older_pr.pr_state = Some("MERGED".to_string());
    let mut newer_pr = row(
        "work-session-pr-precedence",
        Some("work/pr-precedence"),
        "2026-06-12T10:00:00Z",
        1,
        "active",
    );
    newer_pr.pr_number = Some(200);
    newer_pr.pr_url = Some("https://example.test/pull/200".to_string());
    newer_pr.pr_state = Some("OPEN".to_string());
    let mut pr_precedence = vec![older_pr, newer_pr];

    super::super::assign_and_merge_workspace_groups(&mut pr_precedence, &root);

    assert_eq!(pr_precedence[0].pr_number, Some(200));
    assert_eq!(
        pr_precedence[0].pr_url.as_deref(),
        Some("https://example.test/pull/200")
    );
    assert_eq!(
        pr_precedence[0].pr_state.as_deref(),
        Some("OPEN"),
        "newer representative PR metadata must win when it is present"
    );
}

#[test]
fn legacy_workspace_lifecycle_does_not_create_an_implicit_close_target() {
    let legacy = serde_json::json!({
        "id": "work-legacy-parent",
        "title": "Legacy Workspace",
        "status_category": "idle",
        "status_text": "Paused",
        "summary": null,
        "owner": null,
        "next_action": null,
        "active_agents": 0,
        "blocked_agents": 0,
        "branch": "work/legacy",
        "worktree_path": null,
        "pr_number": null,
        "pr_url": null,
        "pr_state": null,
        "board_refs": [],
        "agents": [],
        "lifecycle_state": "paused"
    });

    let workspace: gwt::ActiveWorkItemView =
        serde_json::from_value(legacy).expect("deserialize legacy Workspace row");

    assert!(
        workspace.works.is_empty(),
        "legacy parent lifecycle is display compatibility only and must not invent a child Work"
    );
}

/// SPEC-2359 W16-3 (FR-390): a row whose branch is known only from fetched
/// refs (no recorded worktree, not in the local-worktree set) is flagged
/// `remote_only`; rows with a worktree or a locally checked-out branch and
/// branchless rows are not.
#[test]
fn mark_remote_only_flags_fetched_branches_without_local_worktree() {
    fn row(id: &str, branch: Option<&str>, worktree: Option<&str>) -> gwt::ActiveWorkItemView {
        gwt::ActiveWorkItemView {
            linked_issue_numbers: Vec::new(),
            id: id.to_string(),
            title: id.to_string(),
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
            worktree_path: worktree.map(str::to_string),
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
            updated_at: String::new(),
        }
    }

    let mut local = std::collections::HashSet::new();
    local.insert("work/local".to_string());
    let mut works = vec![
        row("w-remote", Some("origin/work/fetched"), None),
        row("w-local-branch", Some("work/local"), None),
        row("w-with-worktree", Some("work/other"), Some("/tmp/x")),
        row("w-branchless", None, None),
    ];

    super::super::assign_and_merge_workspace_groups(&mut works, Path::new("/repo"));
    super::super::mark_remote_only_active_works(&mut works, Some(&local));

    assert!(works[0].remote_only, "fetched-only branch is Remote");
    assert!(!works[1].remote_only, "locally checked-out branch is not");
    assert!(!works[2].remote_only, "rows with a worktree are not");
    assert!(!works[3].remote_only, "branchless rows are not");
    assert_eq!(works[0].works.len(), 1);
    assert!(!works[0].works[0].manual_close_allowed);
    assert_eq!(
        works[0].works[0].close_blocked_reason.as_deref(),
        Some("remote_environment_unknown")
    );

    let mut unknown = vec![row("w-unknown", Some("work/unknown"), None)];
    super::super::assign_and_merge_workspace_groups(&mut unknown, Path::new("/repo"));
    super::super::mark_remote_only_active_works(&mut unknown, None);

    assert!(
        !unknown[0].remote_only,
        "missing local branch scan data must remain undetermined"
    );
    assert!(unknown[0].works[0].manual_close_allowed);
    assert_eq!(unknown[0].works[0].close_blocked_reason, None);
}

/// Issue #4774: rows sharing a branch pick the newest row as representative by
/// instant, not by RFC3339 text. A second-precision `...56Z` Work record must
/// not outrank a session row stamped `...56.831192+00:00` later in the same
/// second just because `'Z'` sorts after `'.'`.
#[test]
fn workspace_group_representative_is_the_newest_row_by_instant_not_by_text() {
    fn row(id: &str, updated_at: &str) -> gwt::ActiveWorkItemView {
        gwt::ActiveWorkItemView {
            linked_issue_numbers: Vec::new(),
            id: id.to_string(),
            title: id.to_string(),
            status_category: "idle".to_string(),
            status_text: "Paused".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: 0,
            blocked_agents: 0,
            branch: Some("work/off-loop".to_string()),
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
        }
    }

    let mut same_second = vec![
        row("work-session-session-1", "2026-09-30T01:36:56.831192+00:00"),
        row("work-offloop-a1b2c3", "2026-09-30T01:36:56Z"),
    ];
    super::super::assign_and_merge_workspace_groups(&mut same_second, Path::new("/repo"));
    assert_eq!(same_second.len(), 1);
    assert_eq!(same_second[0].id, "work-session-session-1");

    let mut later_record = vec![
        row("work-session-session-1", "2026-09-30T01:36:56.831192+00:00"),
        row("work-offloop-a1b2c3", "2026-09-30T01:36:57Z"),
    ];
    super::super::assign_and_merge_workspace_groups(&mut later_record, Path::new("/repo"));
    assert_eq!(later_record.len(), 1);
    assert_eq!(later_record[0].id, "work-offloop-a1b2c3");
}

/// SPEC-2359 W16-4 (FR-391): merged ∧ stale rows classify as derived Done;
/// activity after the merge reference clears it; explicit terminal closes
/// and pr_state-only merges never enter the derived classification; and the
/// marking writes nothing (US-61).
#[test]
fn mark_merged_classifies_done_equivalent_for_stale_merged_rows() {
    fn row(
        id: &str,
        branch: Option<&str>,
        pr_state: Option<&str>,
        lifecycle: &str,
        updated_at: &str,
    ) -> gwt::ActiveWorkItemView {
        gwt::ActiveWorkItemView {
            linked_issue_numbers: Vec::new(),
            id: id.to_string(),
            title: id.to_string(),
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
            lifecycle_state: lifecycle.to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            updated_at: updated_at.to_string(),
        }
    }

    let merge_at = chrono::Utc::now();
    let stale =
        (merge_at - chrono::Duration::hours(2)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let fresh =
        (merge_at + chrono::Duration::hours(2)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let merged: HashMap<String, chrono::DateTime<chrono::Utc>> =
        [("work/merged".to_string(), merge_at)]
            .into_iter()
            .collect();

    let mut works = vec![
        row("w-stale", Some("work/merged"), None, "paused", &stale),
        row("w-fresh", Some("work/merged"), None, "paused", &fresh),
        row("w-closed", Some("work/merged"), None, "done", &stale),
        row(
            "w-pr-only",
            Some("work/other"),
            Some("MERGED"),
            "paused",
            &stale,
        ),
    ];

    super::super::mark_merged_active_works(&mut works, Some(&merged), None);

    assert!(works[0].done_equivalent, "merged ∧ stale → derived Done");
    assert!(
        !works[1].done_equivalent,
        "updated after the merge → back to Active/Paused (FR-391)"
    );
    assert!(
        !works[2].done_equivalent,
        "explicit terminal close keeps its own lifecycle"
    );
    assert!(
        !works[3].done_equivalent,
        "pr_state stays badge-only — membership rides the scan verdict"
    );
    assert!(works[3].merged_into_base, "pr_state still drives the badge");
}

#[test]
fn mark_cleanup_candidates_exposes_no_changes_reason_without_merged_badge() {
    let mut works = vec![gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: "w-no-changes".to_string(),
        title: "No changes".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/no-changes".to_string()),
        worktree_path: Some("/tmp/gwt-no-changes".to_string()),
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
        updated_at: String::new(),
    }];
    let cleanup_ready: HashMap<String, String> =
        [("work/no-changes".to_string(), "no_changes".to_string())]
            .into_iter()
            .collect();

    super::super::mark_workspace_cleanup_candidates(
        &mut works,
        Some(&cleanup_ready),
        Some(&HashSet::new()),
        &[],
        Some(&HashSet::new()),
    );

    let candidate = works[0]
        .cleanup_candidate
        .as_ref()
        .expect("no-changes branch is cleanup-ready");
    assert_eq!(candidate.branch, "work/no-changes");
    assert_eq!(candidate.reason, "no_changes");
    assert_eq!(works[0].cleanup_blocked_reason, None);
    assert!(
        !works[0].merged_into_base,
        "no-changes cleanup does not claim a merged badge"
    );

    super::super::mark_workspace_cleanup_candidates(
        &mut works,
        Some(&cleanup_ready),
        Some(&HashSet::new()),
        &[],
        None,
    );
    assert_eq!(
        works[0].cleanup_candidate, None,
        "missing process-liveness cache must fail closed"
    );
    assert_eq!(
        works[0].cleanup_blocked_reason.as_deref(),
        Some("process_liveness_unknown")
    );
}

#[test]
fn mark_cleanup_candidates_sets_blocked_reason_for_live_agent_and_process() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let live_process_worktree = temp.path().join("gwt-live-process");
    fs::create_dir_all(&live_process_worktree).expect("create live process worktree");
    let mut works = vec![
        gwt::ActiveWorkItemView {
            linked_issue_numbers: Vec::new(),
            id: "w-live-agent".to_string(),
            title: "Live agent".to_string(),
            status_category: "active".to_string(),
            status_text: "Active".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: 1,
            blocked_agents: 0,
            branch: Some("work/live-agent".to_string()),
            worktree_path: Some("/tmp/gwt-live-agent".to_string()),
            managed_hook_health: None,
            pr_number: None,
            pr_url: None,
            pr_state: Some("MERGED".to_string()),
            board_refs: Vec::new(),
            agents: Vec::new(),
            works: Vec::new(),
            lifecycle_state: "active".to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: true,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            updated_at: String::new(),
        },
        gwt::ActiveWorkItemView {
            linked_issue_numbers: Vec::new(),
            id: "w-live-process".to_string(),
            title: "Live process".to_string(),
            status_category: "idle".to_string(),
            status_text: "Paused".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: 0,
            blocked_agents: 0,
            branch: Some("work/live-process".to_string()),
            worktree_path: Some(live_process_worktree.display().to_string()),
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
            updated_at: String::new(),
        },
    ];
    let cleanup_ready: HashMap<String, String> = [
        ("work/live-agent".to_string(), "pr_merged".to_string()),
        ("work/live-process".to_string(), "no_changes".to_string()),
    ]
    .into_iter()
    .collect();
    let session = ActiveAgentSession {
        window_id: "window-live-agent".to_string(),
        session_id: "session-live-agent".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/live-agent".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: PathBuf::from("/tmp/gwt-live-agent"),
        agent_project_root: "/tmp/gwt-live-agent".to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    let live_process_branches = HashSet::from(["work/live-process".to_string()]);

    super::super::mark_workspace_cleanup_candidates(
        &mut works,
        Some(&cleanup_ready),
        Some(&HashSet::new()),
        &[&session],
        Some(&live_process_branches),
    );

    assert_eq!(works[0].cleanup_candidate, None);
    assert_eq!(
        works[0].cleanup_blocked_reason.as_deref(),
        Some("live_agent")
    );
    assert_eq!(works[1].cleanup_candidate, None);
    assert_eq!(
        works[1].cleanup_blocked_reason.as_deref(),
        Some("live_process")
    );
}

// SPEC-3075: the rail "what work was running" summary derivation. Surfaces the
// agent-declared title-summary purpose, with a fallback chain that skips
// identifier-shaped titles (skill names / work ids / UUIDs) and the branch.
#[test]
fn derive_work_summary_prefers_live_agent_title_summary() {
    let summary = super::super::derive_work_summary(
        Some("Work 要約を目的第一に再構成"),
        Some("journal purpose"),
        Some("gwt-manage-pr"),
        Some("work/20260612-1405"),
    );
    assert_eq!(summary.as_deref(), Some("Work 要約を目的第一に再構成"));
}

#[test]
fn derive_work_summary_falls_back_to_journal_then_recorded_title() {
    // No live agent purpose -> recorded journal purpose wins.
    assert_eq!(
        super::super::derive_work_summary(
            None,
            Some("journal recorded purpose"),
            Some("gwt-build-spec"),
            Some("work/x"),
        )
        .as_deref(),
        Some("journal recorded purpose"),
    );
    // No agent / journal -> a real (non-identifier) recorded title wins.
    assert_eq!(
        super::super::derive_work_summary(None, None, Some("Release Notes cleanup"), None)
            .as_deref(),
        Some("Release Notes cleanup"),
    );
}

#[test]
fn derive_work_summary_is_none_when_no_declared_purpose() {
    // A skill-name title is not a declared purpose -> None (the caller then
    // fills from the branch tip commit subject). The owner is NOT folded in.
    assert_eq!(
        super::super::derive_work_summary(None, None, Some("gwt-manage-pr"), None),
        None,
    );
    // Backfill Work: title == branch -> None (UI labels by branch).
    assert_eq!(
        super::super::derive_work_summary(
            None,
            None,
            Some("work/20260614-0444"),
            Some("work/20260614-0444"),
        ),
        None,
    );
    // A raw work-item id title is not a purpose.
    assert_eq!(
        super::super::derive_work_summary(
            None,
            None,
            Some("work-work-20260601-0908-9ffe416f"),
            Some("work/20260601-0908"),
        ),
        None,
    );
}

#[test]
fn apply_work_summary_external_sources_prefers_pr_then_ai_then_commit_subject() {
    use std::collections::HashMap;
    let mut tip_subjects: HashMap<String, String> = HashMap::new();
    tip_subjects.insert(
        "work/20260614-0444".to_string(),
        "feat(workspace): purpose-first rail".to_string(),
    );
    tip_subjects.insert(
        "work/20260610-0907".to_string(),
        "work/20260610-0907".to_string(), // subject == branch -> not a purpose
    );
    // SPEC-3075 FR-006: an AI-polished summary wins over the raw commit subject
    // for a gap row (no PR, no title-summary).
    tip_subjects.insert(
        "work/20260609-1130".to_string(),
        "Merge pull request #42 from x".to_string(), // noisy raw subject
    );
    tip_subjects.insert(
        "work/20260617-0417".to_string(),
        "Merge pull request #3102 from akiojin/work/20260616-1443".to_string(),
    );
    tip_subjects.insert(
        "work/20260617-0250".to_string(),
        "chore(release): v9.61.0".to_string(),
    );
    let mut ai_summaries: HashMap<String, String> = HashMap::new();
    ai_summaries.insert(
        "work/20260609-1130".to_string(),
        "tray の Copy URL のちらつきを修正".to_string(),
    );
    let mut pr_titles: HashMap<String, String> = HashMap::new();
    // A PR title overrides even a declared title-summary already in work_summary.
    pr_titles.insert(
        "work/20260612-1405".to_string(),
        "Surface work purpose in the Workspace rail".to_string(),
    );
    pr_titles.insert(
        "work/20260617-0422".to_string(),
        "Merge pull request #3102 from akiojin/work/20260616-1443".to_string(),
    );

    let base = |branch: &str, work_summary: Option<&str>| gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
        id: branch.to_string(),
        title: branch.to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: work_summary.map(str::to_string),
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
        board_refs: vec![],
        agents: vec![],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        updated_at: String::new(),
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
    };
    let mut works = vec![
        base("work/20260614-0444", None), // no PR, gap -> filled by commit subject
        base("work/20260612-1405", Some("Keep my purpose")), // PR title overrides title-summary
        base("work/20260610-0907", None), // no PR, subject == branch -> stays None
        base("work/20260609-1130", None), // no PR, AI summary beats noisy commit subject
        base("work/20260617-0417", None), // no AI, noisy merge subject -> no purpose
        base("work/20260617-0250", None), // no AI, release bump subject -> no purpose
        base("work/20260617-0422", Some("Declared purpose")), // noisy PR title must not override
    ];
    super::super::apply_work_summary_external_sources(
        &mut works,
        Some(&pr_titles),
        Some(&ai_summaries),
        Some(&tip_subjects),
    );
    assert_eq!(
        works[0].work_summary.as_deref(),
        Some("feat(workspace): purpose-first rail"),
    );
    assert_eq!(
        works[1].work_summary.as_deref(),
        Some("Surface work purpose in the Workspace rail"),
        "PR title overrides the declared title-summary",
    );
    assert_eq!(works[2].work_summary, None);
    assert_eq!(
        works[3].work_summary.as_deref(),
        Some("tray の Copy URL のちらつきを修正"),
        "AI-polished summary wins over the raw commit subject",
    );
    assert_eq!(
        works[4].work_summary, None,
        "raw merge commit subjects are git mechanics, not Work purpose",
    );
    assert_eq!(
        works[5].work_summary, None,
        "raw release bump subjects are git mechanics, not Work purpose",
    );
    assert_eq!(
        works[6].work_summary.as_deref(),
        Some("Declared purpose"),
        "noisy external titles must not override a declared purpose",
    );
}

#[test]
fn is_summary_noise_flags_merge_and_release_commits() {
    assert!(super::super::is_summary_noise(""));
    assert!(super::super::is_summary_noise(
        "Merge pull request #3078 from akiojin/work/x"
    ));
    assert!(super::super::is_summary_noise("Merge branch 'develop'"));
    assert!(super::super::is_summary_noise(
        "Merge remote-tracking branch 'origin/develop'"
    ));
    assert!(super::super::is_summary_noise("chore(release): v9.58.0"));
    assert!(super::super::is_summary_noise(
        "chore: merge origin/develop"
    ));
    // Real work is not noise.
    assert!(!super::super::is_summary_noise(
        "feat(workspace): purpose-first rail"
    ));
    assert!(!super::super::is_summary_noise(
        "fix: reveal reused surface tabs"
    ));
}

#[test]
fn is_identifier_like_title_classifies_shapes() {
    assert!(super::super::is_identifier_like_title("gwt-manage-pr"));
    assert!(super::super::is_identifier_like_title("gwt-build-spec"));
    assert!(super::super::is_identifier_like_title(
        "work-work-20260601-0908-9ffe416f"
    ));
    assert!(super::super::is_identifier_like_title(
        "550e8400-e29b-41d4-a716-446655440000"
    ));
    // Real purposes are not identifiers.
    assert!(!super::super::is_identifier_like_title(
        "Release Notes cleanup"
    ));
    assert!(!super::super::is_identifier_like_title(
        "Work 要約を目的第一に再構成"
    ));
    assert!(!super::super::is_identifier_like_title("SPEC-3075"));
    assert!(!super::super::is_identifier_like_title("develop"));
}

#[test]
fn issue_monitor_launch_succeeded_ack_is_non_scanning_and_persists() {
    // Issue #3222: the launch-success ACK used to re-enter the full
    // scan+claim flow on a fresh disk snapshot that could not see other
    // in-flight claims, re-claiming them (same-owner renewal) and spawning
    // duplicate windows past max_active. The ACK must only bind the window and
    // persist; scanning for a fresh snapshot is allowed, claiming is not.
    let temp = tempfile::TempDir::new().expect("tempdir");
    // Thread-local override: never mutate process-global HOME in parallel tests.
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    // Seed an in-flight claim (Launching, no window bound yet) on disk.
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
            issue_number: 42,
            claimed_at: None,
        }],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let (mut runtime, _recorded) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.issue_monitor_launch_succeeded_and_drain(&repo, 42, "tab-1::agent-1");

    // The ACK may scan for a fresh snapshot, but must NOT claim/launch: no
    // settings-required wizard, no "launch requested" toast.
    for event in &events {
        assert!(
            !matches!(event.event, BackendEvent::LaunchWizardState { .. }),
            "ACK must not open the launch wizard (settings-required prompt)"
        );
        if let BackendEvent::IssueMonitorToast { message, .. } = &event.event {
            assert!(
                !message.contains("launch requested"),
                "ACK must not trigger launches: {message}"
            );
        }
    }
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload");
    assert!(
        persisted
            .launched_issues
            .iter()
            .any(|entry| entry.issue_number == 42 && entry.window_id == "tab-1::agent-1"),
        "the ACK binds and persists the window: {:?}",
        persisted.launched_issues
    );
    assert!(
        persisted.launching_issues.is_empty(),
        "the in-flight marker is consumed by the bind"
    );
}

#[test]
fn issue_monitor_windows_closed_requeue_is_non_scanning() {
    // Issue #3222 (same re-entrancy class): closing a monitor window requeues
    // + persists and may rescan for the snapshot, but must not claim/launch.
    let temp = tempfile::TempDir::new().expect("tempdir");
    // Thread-local override: never mutate process-global HOME in parallel tests.
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "tab-1::agent-1".to_string(),
        }],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events =
        runtime.issue_monitor_windows_closed_events(&repo, &["tab-1::agent-1".to_string()]);

    // Close may scan for a fresh snapshot, but must NOT claim/launch — a
    // re-claim here could instantly respawn the just-closed window or
    // duplicate other in-flight launches.
    for event in &events {
        assert!(
            !matches!(event.event, BackendEvent::LaunchWizardState { .. }),
            "window close must not open the launch wizard"
        );
        if let BackendEvent::IssueMonitorToast { message, .. } = &event.event {
            assert!(
                !message.contains("launch requested"),
                "window close must not trigger launches: {message}"
            );
        }
    }
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload");
    assert!(
        persisted.launched_issues.is_empty(),
        "closed window is released from the launched set"
    );
}

/// Issue #3783: socket publication and the local prefs fallback may wait on
/// transport or file locks, so a pane.close acknowledgement must enqueue that
/// lifecycle work with the PTY finalizer instead of executing it on Tao.
#[test]
fn issue_monitor_close_ack_defers_control_transport_to_finalizer() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "tab-1::agent-1".to_string(),
        }],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let started = Instant::now();
    let events = runtime.close_window_events("tab-1::agent-1");
    assert!(started.elapsed() < Duration::from_millis(300));
    assert!(!events.is_empty());
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("queued prefs")
            .launched_issues
            .len(),
        1,
        "the acknowledgement path must not enter daemon or prefs I/O"
    );

    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued monitor close finalizer");
    finalizer();
    assert!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("finalized prefs")
            .launched_issues
            .is_empty(),
        "the background finalizer must release the monitor slot"
    );
}

/// Issue #3627: a launch whose agent window is gone from the tab canvas held
/// its `max_active` slot forever, because release was driven exclusively by PTY
/// exit events. A window reaped without one — app restart, crash, error pane
/// closed outside `close_window_events` — told the Monitor nothing, and with
/// every slot held the queue stopped permanently (95 queued behind 11 launches
/// against a cap of 5, all last seen more than a day earlier).
#[test]
fn a_launch_whose_agent_window_vanished_releases_its_slot_on_the_scheduled_tick() {
    // Issue #3609: the scheduled tick's worker re-resolves the prefs path from
    // the process-global HOME on its own thread, so this test pins HOME under
    // the shared process env lock rather than a thread-local `ScopedGwtHome`.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab-1::agent-24".to_string(),
            }],
            launch_confirmations: std::collections::BTreeMap::from([(
                42,
                gwt::issue_monitor::IssueMonitorLaunchConfirmation {
                    window_id: "tab-1::agent-24".to_string(),
                    claim_id: None,
                    delivery_id: None,
                    confirmed_at: "2026-08-17T08:55:00Z".to_string(),
                },
            )]),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");

    // The canvas kept a different agent window: the launched one is gone.
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-51",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _recorded) =
        sample_runtime_with_events(temp.path(), vec![tab.clone()], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.issue_monitor_scheduled_tick_events_at("2026-08-17T08:59:59Z");
    tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued registration-pending worker")();
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .launched_issues
            .len(),
        1,
        "a scheduled disappearance must wait for registration"
    );

    // A new observer after the grace boundary must still reclaim a dead slot.
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.issue_monitor_scheduled_tick_events_at("2026-08-17T09:00:00Z");

    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("prefs before worker")
            .launched_issues
            .len(),
        1,
        "the Tao tick must only snapshot live window ids and enqueue work"
    );
    tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued scheduled worker")();

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload");
    assert!(
        persisted.launched_issues.is_empty(),
        "a window the canvas no longer has cannot keep holding a slot"
    );
}

/// Issue #4328: a queued scan's canvas predates a legitimate launch ACK.
/// Reading newer prefs must not turn that old absence into an exact close.
#[test]
fn issue_4328_scheduled_scan_preserves_a_launch_registered_after_canvas_capture() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo_without_origin(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 1,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled monitor");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-51",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.issue_monitor_scheduled_tick_events_at("2026-09-14T13:43:32Z");

    let fresh = runtime
        .tab_mut("tab-1")
        .expect("project tab")
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    assert_ne!(fresh.id, "agent-51", "the second specimen uses a fresh id");
    runtime.register_window("tab-1", &fresh.id);
    runtime.set_window_status("tab-1", &fresh.id, WindowProcessStatus::Running);
    let window_id = combined_window_id("tab-1", &fresh.id);
    runtime.issue_monitor_launch_succeeded_and_drain(&repo, 4258, &window_id);
    let before = gwt::load_issue_monitor_prefs(&prefs_path).expect("ACKed prefs");
    assert_eq!(
        before.launched_issues.len(),
        1,
        "normal ACK reserves the slot"
    );

    tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued scheduled worker")();

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("prefs after scan");
    assert_eq!(
        persisted.launched_issues, before.launched_issues,
        "a stale canvas cannot release the fresh #4258 launch"
    );
    assert!(runtime.tracked_window_exists(&window_id));
    let mut monitor =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), persisted);
    monitor.set_gui_connected(true);
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: 4328,
        title: "Next queued issue".to_string(),
        labels: vec!["auto-improve".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    });
    assert!(
        monitor
            .next_launch_request("2026-09-14T13:44:37Z")
            .is_none(),
        "the live launch leaves no capacity for #4328"
    );
}

/// Issue #3627 (companion to the release above): the reclaim must not fire on
/// a live agent. Everything an agent can stall on — an approval prompt, a
/// provider rate limit, a genuine hang — leaves the window in place, and
/// SPEC-3431 FR-069 deliberately keeps those slots held so the PM decides.
#[test]
fn a_launch_whose_agent_window_still_exists_keeps_its_slot_on_the_scheduled_tick() {
    // Issue #3609: see the companion test above — the scan worker reads the
    // process-global HOME from another thread.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab-1::agent-24".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-24",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _recorded) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.issue_monitor_scheduled_tick_events_at("2026-08-17T09:00:00Z");

    tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued scheduled worker")();

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload");
    assert_eq!(
        persisted
            .launched_issues
            .iter()
            .map(|launched| launched.issue_number)
            .collect::<Vec<_>>(),
        vec![42],
        "a window that still exists keeps its slot however long it has been quiet"
    );
}

#[test]
fn issue_monitor_launch_success_is_persisted_to_the_window_owner_project() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo A");
    std::fs::create_dir_all(&repo_b).expect("create repo B");
    init_repo_without_origin(&repo_a);
    init_repo_without_origin(&repo_b);

    let prefs_a_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo_a);
    let prefs_b_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo_b);
    gwt::save_issue_monitor_prefs(
        &prefs_a_path,
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 900,
                window_id: "project-a::sentinel".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed repo A prefs");
    gwt::save_issue_monitor_prefs(
        &prefs_b_path,
        &gwt::IssueMonitorPrefs {
            launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: None,
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed repo B prefs");

    let tab_a = sample_project_tab_with_window_at(
        "project-a",
        "sentinel",
        repo_a.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let tab_b = sample_project_tab_with_window_at(
        "project-b",
        "agent-1",
        repo_b.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab_a, tab_b], Some("project-a"));

    assert_eq!(
        runtime
            .issue_monitor_tab_id_for_project_root(&repo_b)
            .as_deref(),
        Some("project-b"),
        "daemon launch/review routing resolves the owner tab without switching active tabs"
    );
    let events =
        runtime.issue_monitor_launch_succeeded_and_drain(&repo_b, 42, "project-b::agent-1");

    assert!(
        events.iter().all(|event| matches!(&event.target, DispatchTarget::Project(key) if key == &runtime.project_context("project-b").expect("owner context").project_key)),
        "launch ACK events belong only to the owner project"
    );

    assert_eq!(
        runtime.issue_monitor_issue_number_for_window(&repo_b, "project-b::agent-1"),
        Some(42),
        "heartbeat routing resolves a launched mapping after pending feedback is consumed"
    );
    let prefs_a = gwt::load_issue_monitor_prefs(&prefs_a_path).expect("reload repo A");
    assert_eq!(
        prefs_a.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 900,
            window_id: "project-a::sentinel".to_string(),
        }],
        "the active project must remain unchanged"
    );
    let prefs_b = gwt::load_issue_monitor_prefs(&prefs_b_path).expect("reload repo B");
    assert_eq!(
        prefs_b.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "project-b::agent-1".to_string(),
        }],
        "the launch ACK belongs to the window owner project"
    );
    assert!(prefs_b.launching_issues.is_empty());
}

#[test]
fn closing_an_inactive_project_window_requeues_only_its_owner_project() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo A");
    std::fs::create_dir_all(&repo_b).expect("create repo B");
    init_repo_without_origin(&repo_a);
    init_repo_without_origin(&repo_b);

    let prefs_a_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo_a);
    let prefs_b_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo_b);
    gwt::save_issue_monitor_prefs(
        &prefs_a_path,
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 900,
                window_id: "project-a::agent-1".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed repo A prefs");
    gwt::save_issue_monitor_prefs(
        &prefs_b_path,
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "project-b::agent-1".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed repo B prefs");

    let tab_a = sample_project_tab_with_window_at(
        "project-a",
        "agent-1",
        repo_a.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let tab_b = sample_project_tab_with_window_at(
        "project-b",
        "agent-1",
        repo_b.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab_a, tab_b], Some("project-a"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let events = runtime.close_window_events("project-b::agent-1");

    assert!(
        events.iter().all(|event| matches!(&event.target, DispatchTarget::Project(key) if key == &runtime.project_context("project-b").expect("owner context").project_key)),
        "close events belong only to the owner project"
    );
    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued inactive-project close finalizer");
    finalizer();
    let completion = apply_recorded_window_close_finalized(&mut runtime, &recorded_events);
    assert!(completion.iter().all(|event| matches!(&event.target, DispatchTarget::Project(key) if key == &runtime.project_context("project-b").expect("owner context").project_key)));

    let prefs_a = gwt::load_issue_monitor_prefs(&prefs_a_path).expect("reload repo A");
    assert_eq!(
        prefs_a.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 900,
            window_id: "project-a::agent-1".to_string(),
        }],
        "closing another project's window must not consume the active project's slot"
    );
    let prefs_b = gwt::load_issue_monitor_prefs(&prefs_b_path).expect("reload repo B");
    assert!(
        prefs_b.launched_issues.is_empty(),
        "the owner project's launched slot is released"
    );
}

#[test]
fn closing_an_inactive_project_tab_requeues_its_owned_monitor_windows() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo A");
    std::fs::create_dir_all(&repo_b).expect("create repo B");
    init_repo_without_origin(&repo_a);
    init_repo_without_origin(&repo_b);

    let prefs_b_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo_b);
    gwt::save_issue_monitor_prefs(
        &prefs_b_path,
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "project-b::agent-1".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed repo B prefs");

    let tab_a = sample_project_tab("project-a", "Repo A", repo_a, ProjectKind::Git, &[]);
    let tab_b = sample_project_tab_with_window_at(
        "project-b",
        "agent-1",
        repo_b,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab_a, tab_b], Some("project-a"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let events = runtime.close_project_tab_events("project-b");

    assert!(events.iter().all(|event| !matches!(
        event.event,
        BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
    )));
    let before_finalizer =
        gwt::load_issue_monitor_prefs(&prefs_b_path).expect("reload repo B before finalizer");
    assert_eq!(
        before_finalizer.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "project-b::agent-1".to_string(),
        }],
        "the accepted tab close does not perform durable monitor I/O on the Tao thread"
    );

    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued inactive-project tab close finalizer");
    finalizer();
    let completion = apply_recorded_window_close_finalized(&mut runtime, &recorded_events);
    assert!(completion.iter().all(|event| !matches!(
        event.event,
        BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
    )));

    let prefs_b = gwt::load_issue_monitor_prefs(&prefs_b_path).expect("reload repo B");
    assert!(
        prefs_b.launched_issues.is_empty(),
        "closing an inactive tab releases its own launched slot"
    );
}

#[test]
fn issue_monitor_agent_failure_is_persisted_to_the_window_owner_project() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo A");
    std::fs::create_dir_all(&repo_b).expect("create repo B");
    init_repo_without_origin(&repo_a);
    init_repo_without_origin(&repo_b);

    let prefs_a_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo_a);
    let prefs_b_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo_b);
    gwt::save_issue_monitor_prefs(
        &prefs_a_path,
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 900,
                window_id: "project-a::agent-1".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed repo A prefs");
    gwt::save_issue_monitor_prefs(
        &prefs_b_path,
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "project-b::agent-1".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed repo B prefs");

    let tab_a = sample_project_tab_with_window_at(
        "project-a",
        "agent-1",
        repo_a.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let tab_b = sample_project_tab_with_window_at(
        "project-b",
        "agent-1",
        repo_b.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab_a, tab_b], Some("project-a"));
    runtime.pending_launch_feedback_contexts.insert(
        "project-b::agent-1".to_string(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: Some(repo_b.clone()),
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );

    let events = runtime.handle_runtime_status_with_exit_confirmation(
        "project-b::agent-1".to_string(),
        WindowProcessStatus::Error,
        Some("agent failed".to_string()),
        true,
    );

    assert!(
        events.iter().all(|event| matches!(&event.target, DispatchTarget::Project(key) if key == &runtime.project_context("project-b").expect("owner context").project_key)),
        "an inactive agent failure must not replace the active project's UI"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::IssueMonitorToast { .. })),
        "the owner-routed failure still notifies the operator"
    );

    let prefs_a = gwt::load_issue_monitor_prefs(&prefs_a_path).expect("reload repo A");
    assert!(
        prefs_a.failed_issues.is_empty(),
        "the active project must not receive another project's failure"
    );
    assert_eq!(prefs_a.launched_issues[0].issue_number, 900);
    let prefs_b = gwt::load_issue_monitor_prefs(&prefs_b_path).expect("reload repo B");
    assert_eq!(prefs_b.failed_issues.len(), 1);
    assert_eq!(prefs_b.failed_issues[0].issue_number, 42);
    assert_eq!(prefs_b.failed_issues[0].message, "agent failed");
}

#[test]
fn pm_cleanup_holds_the_shared_lifecycle_lock_through_probe_and_remove() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let codex_home = temp.path().join(".codex");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = create_detached_pm_worktree_fixture(&repo);
    let codex_config_path = codex_home.join("config.toml");
    let codex_project =
        gwt_skills::register_codex_managed_project_trust(&worktree, &codex_config_path)
            .expect("seed PM Codex project trust")
            .project_path
            .to_string_lossy()
            .into_owned();
    fs::write(
        worktree.join("cleanup-barrier.txt"),
        "generated cleanup probe\n",
    )
    .expect("write cleanup barrier entry");
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(&repo);
    fs::create_dir_all(project_dir.join("project-state")).expect("create project state");
    let lock_path = project_dir.join("project-state/pm-refresh.lock");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker_repo = repo.clone();
    let worker_gwt_home = temp.path().join(".gwt");
    let worker = thread::spawn(move || {
        let _worker_home = ScopedGwtHome::set(worker_gwt_home);
        gwt::pm_registry::cleanup_pm_worktree_for_repo_path(&worker_repo, |_worktree, entry| {
            if entry == "cleanup-barrier.txt" {
                entered_tx.send(()).expect("signal cleanup probe");
                release_rx.recv().expect("release cleanup probe");
                true
            } else {
                false
            }
        })
    });
    if let Err(error) = entered_rx.recv_timeout(Duration::from_secs(5)) {
        panic!(
            "cleanup must reach its local-work probe ({error:?}); worker={:?}",
            worker.join()
        );
    }
    let competing_lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .expect("open competing PM lifecycle lock");

    assert!(
        competing_lock.try_lock_exclusive().is_err(),
        "refresh must not acquire the shared lock while cleanup is probing"
    );
    release_tx.send(()).expect("release cleanup worker");
    let outcome = worker
        .join()
        .expect("join cleanup worker")
        .expect("cleanup");

    assert_eq!(outcome, gwt::pm_registry::PmWorktreeCleanupOutcome::Removed);
    assert!(
        !worktree.exists(),
        "serialized cleanup must remove the worktree"
    );
    assert_eq!(
        codex_project_trust_level(&codex_config_path, &codex_project),
        None,
        "PM worktree removal must revoke its Codex project trust"
    );
}

#[test]
fn pm_cleanup_keeps_worktree_when_codex_trust_revocation_fails() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let codex_home = temp.path().join(".codex");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = create_detached_pm_worktree_fixture(&repo);
    let config_path = codex_home.join("config.toml");
    fs::create_dir_all(&codex_home).expect("create Codex home");
    fs::write(&config_path, "projects = [\n").expect("write malformed Codex config");

    let error = gwt::pm_registry::cleanup_pm_worktree_for_repo_path(&repo, |_, _| false)
        .expect_err("malformed Codex config must stop PM cleanup");

    assert!(
        error.to_string().contains("Codex project trust"),
        "failure must identify trust revocation: {error}"
    );
    assert!(
        worktree.exists(),
        "trust revocation failure must keep the PM worktree"
    );
    assert_eq!(
        fs::read_to_string(config_path).expect("malformed config remains"),
        "projects = [\n"
    );
}

#[test]
fn pm_cleanup_retains_project_trust_when_local_work_keeps_the_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let codex_home = temp.path().join(".codex");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = create_detached_pm_worktree_fixture(&repo);
    let config_path = codex_home.join("config.toml");
    let project_key = gwt_skills::register_codex_managed_project_trust(&worktree, &config_path)
        .expect("seed PM Codex project trust")
        .project_path
        .to_string_lossy()
        .into_owned();
    fs::write(worktree.join("user-work.txt"), "keep me\n").expect("write local PM work");

    let outcome = gwt::pm_registry::cleanup_pm_worktree_for_repo_path(&repo, |_, _| false)
        .expect("local-work classification");

    assert_eq!(
        outcome,
        gwt::pm_registry::PmWorktreeCleanupOutcome::RetainedLocalWork
    );
    assert!(worktree.exists(), "local work must keep the PM worktree");
    assert_eq!(
        codex_project_trust_level(&config_path, &project_key).as_deref(),
        Some("trusted"),
        "retained worktree must keep its project trust"
    );
}

#[test]
fn pm_ensure_focuses_live_pm_instead_of_spawning() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    // AS4 / FR-001 positive: a live registered PM must not be duplicated.
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
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1".to_string();
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "pm-session-live".to_string();
    runtime.active_agent_sessions.insert(window_id, session);

    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &repo),
        |_| false,
    )
    .expect("seed registration");

    let windows_before = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .len();

    let events =
        runtime.ensure_pm_agent_for_tab("tab-1", super::super::pm::PmEnsureTrigger::Automatic);

    let windows_after = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .len();
    assert_eq!(
        windows_before, windows_after,
        "live PM must not spawn a duplicate pane"
    );
    assert!(
        !events.is_empty(),
        "ensure focuses the live PM (focus/broadcast events)"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_launches
        .is_empty());
}

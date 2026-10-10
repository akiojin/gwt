use super::*;

#[test]
fn app_runtime_post_board_entry_persists_reply_topics_and_owners() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let parent = post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Question,
            "Can someone verify this?",
            None,
            None,
            vec!["coordination".to_string()],
            vec!["2018".to_string()],
        ),
    )
    .expect("seed board parent")
    .board
    .entries
    .into_iter()
    .next()
    .expect("parent entry");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::PostBoardEntry {
            id: window_id.clone(),
            entry_kind: BoardEntryKind::Next,
            body: "I will take the next slice".to_string(),
            title: None,
            target_workspace: None,
            broadcast: false,
            parent_id: Some(parent.id.clone()),
            topics: vec!["coordination".to_string(), "phase-1b".to_string()],
            owners: vec!["2018".to_string()],
            targets: Vec::new(),
            mentions: vec![
                BoardMention::new(BoardMentionTargetKind::User, "akiojin").with_label("Akio")
            ],
        },
    );

    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardEntries { id, entries, .. },
            ..
        } if client_id == "client-1"
            && id == &window_id
            && entries.iter().any(|entry|
                entry.body == "I will take the next slice"
                && entry.parent_id.as_deref() == Some(parent.id.as_str())
                && entry.related_topics == vec!["coordination".to_string(), "phase-1b".to_string()]
                && entry.related_owners == vec!["2018".to_string()]
                && entry.mentions.len() == 1
                && entry.mentions[0].typed_key() == "user:akiojin"
            )
    )));

    let snapshot = load_snapshot(&repo).expect("load board snapshot");
    assert!(snapshot
        .board
        .entries
        .iter()
        .any(|entry| entry.body == "I will take the next slice"
            && entry.parent_id.as_deref() == Some(parent.id.as_str())
            && entry.related_topics == vec!["coordination".to_string(), "phase-1b".to_string()]
            && entry.related_owners == vec!["2018".to_string()]
            && entry.mentions.len() == 1
            && entry.mentions[0].typed_key() == "user:akiojin"));
}

#[test]
fn app_runtime_post_board_entry_accepts_reply_to_history_parent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let parent_id = "history-parent".to_string();
    let events_path = coordination_events_path(&repo);
    fs::create_dir_all(
        events_path
            .parent()
            .expect("coordination event log has parent"),
    )
    .expect("create coordination dir");
    let mut events = fs::File::create(&events_path).expect("create legacy event log");
    for idx in 0..505 {
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            format!("history entry {idx}"),
            None,
            None,
            vec![],
            vec![],
        );
        if idx == 0 {
            entry.id = parent_id.clone();
        }
        serde_json::to_writer(&mut events, &CoordinationEvent::MessageAppended { entry })
            .expect("write board seed event");
        events.write_all(b"\n").expect("write board seed newline");
    }
    events.flush().expect("flush board seed events");
    let snapshot = load_snapshot(&repo).expect("load board snapshot");
    assert!(!snapshot
        .board
        .entries
        .iter()
        .any(|entry| entry.id == parent_id));
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::PostBoardEntry {
            id: window_id.clone(),
            entry_kind: BoardEntryKind::Next,
            body: "Reply to older context".to_string(),
            title: None,
            target_workspace: None,
            broadcast: false,
            parent_id: Some(parent_id.clone()),
            topics: vec![],
            owners: vec![],
            targets: Vec::new(),
            mentions: Vec::new(),
        },
    );

    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardEntries { id, entries, .. },
            ..
        } if client_id == "client-1"
            && id == &window_id
            && entries.iter().any(|entry|
                entry.body == "Reply to older context"
                && entry.parent_id.as_deref() == Some(parent_id.as_str())
            )
    )));
}

#[test]
fn app_runtime_post_board_entry_without_origin_remains_board_only() {
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
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::PostBoardEntry {
            id: window_id,
            entry_kind: BoardEntryKind::Next,
            body: "Run final verification".to_string(),
            title: None,
            target_workspace: None,
            broadcast: false,
            parent_id: None,
            topics: vec!["start-work".to_string()],
            owners: vec!["SPEC-2359".to_string()],
            targets: Vec::new(),
            mentions: Vec::new(),
        },
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    assert_eq!(projection.next_action, None);
    assert_eq!(projection.owner, None);
    assert!(projection.board_refs.is_empty());
    assert!(
        gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load work items")
            .is_none(),
        "an originless Board entry must not create Work history"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardEntries { .. },
            ..
        } if client_id == "client-1"
    )));
}

#[test]
fn app_runtime_board_milestone_from_unassigned_origin_does_not_create_workspace_history() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "session-unassigned".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Investigate Workspace materialization".to_string()),
            title_summary: Some("Work materialization".to_string()),
            worktree_path: None,
            branch: Some("work/unassigned".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let entry = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Claim,
        "Unassigned claim without materialization must not pollute current Workspace history.",
        None,
        None,
        vec!["workspace-materialization".to_string()],
        vec!["2359".to_string()],
    )
    .with_title_summary("Work materialization")
    .with_origin_session_id("session-unassigned");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &entry);

    let saved = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    let agent = saved
        .agents
        .iter()
        .find(|agent| agent.session_id == "session-unassigned")
        .expect("agent");
    assert_eq!(
        agent.affiliation_status,
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
    );
    assert!(
        gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load workspace history")
            .is_none(),
        "Unassigned origin Board entries must not append to unrelated Workspace history"
    );
}

#[test]
fn app_runtime_board_milestone_uses_latest_duplicate_session_assignment() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let old_at = chrono::Utc::now() - chrono::Duration::minutes(2);
    let current_at = chrono::Utc::now() - chrono::Duration::minutes(1);
    let current_worktree = repo.join("feature-current");
    fs::create_dir_all(&current_worktree).expect("create current Work worktree");
    let stale = gwt_core::workspace_projection::WorkspaceAgentSummary {
        session_id: "session-duplicate-assigned".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
        current_focus: Some("stale".to_string()),
        title_summary: None,
        worktree_path: None,
        branch: Some("feature/stale".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: old_at,
    };
    let mut assigned = stale.clone();
    assigned.current_focus = Some("current".to_string());
    assigned.branch = Some("feature/current".to_string());
    assigned.worktree_path = Some(current_worktree.clone());
    assigned.affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned;
    assigned.workspace_id = Some("work-current-duplicate".to_string());
    assigned.updated_at = current_at;
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.append(&mut vec![stale, assigned]);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        "work-current-duplicate",
        Some("Current Work"),
        None,
        None,
        &[],
        Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("feature/current".to_string()),
                worktree_path: Some(current_worktree),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        ),
        Some("session-duplicate-assigned"),
        current_at,
    )
    .expect("seed current Work");
    let entry = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Current assigned Session milestone.",
        None,
        None,
        vec!["workspace-assignment".to_string()],
        vec!["2359".to_string()],
    )
    .with_origin_session_id("session-duplicate-assigned")
    .with_origin_branch("feature/current");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &entry);

    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work history")
        .expect("Work history");
    let current = works
        .work_items
        .iter()
        .find(|item| item.id == "work-current-duplicate")
        .expect("current assigned Work");
    assert!(
        current.board_refs.iter().any(|id| id == &entry.id),
        "the latest assigned duplicate row must receive the Board event"
    );
}

#[test]
fn app_runtime_old_board_milestone_does_not_rewind_latest_assigned_work_or_agent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let stale_agent_at = chrono::Utc::now() - chrono::Duration::minutes(4);
    let assigned_agent_at = chrono::Utc::now() - chrono::Duration::minutes(3);
    let replayed_at = chrono::Utc::now() - chrono::Duration::minutes(2);
    let current_work_at = chrono::Utc::now() - chrono::Duration::minutes(1);
    let assigned = gwt_core::workspace_projection::WorkspaceAgentSummary {
        session_id: "session-duplicate-assigned".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked,
        current_focus: Some("Current blocked work".to_string()),
        title_summary: None,
        worktree_path: None,
        branch: Some("feature/current".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("work-current-duplicate".to_string()),
        updated_at: assigned_agent_at,
    };
    let mut stale = assigned.clone();
    stale.current_focus = Some("Stale work".to_string());
    stale.branch = Some("feature/stale".to_string());
    stale.affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
    stale.workspace_id = None;
    stale.updated_at = stale_agent_at;
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.append(&mut vec![stale, assigned]);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        "work-current-duplicate",
        Some("Current Work"),
        None,
        None,
        &[],
        None,
        Some("session-duplicate-assigned"),
        current_work_at,
    )
    .expect("seed current Work");
    let mut replayed = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Old assigned Session milestone.",
        None,
        None,
        vec!["workspace-assignment".to_string()],
        vec!["stale-owner".to_string()],
    )
    .with_origin_session_id("session-duplicate-assigned");
    replayed.updated_at = replayed_at;

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &replayed);

    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work history")
        .expect("Work history");
    let current = works
        .work_items
        .iter()
        .find(|item| item.id == "work-current-duplicate")
        .expect("current assigned Work");
    assert_eq!(
        current.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
    );
    assert_eq!(current.title, "Current Work");
    assert_eq!(current.updated_at, current_work_at);
    assert!(!current.board_refs.iter().any(|id| id == &replayed.id));
    let current_projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load current projection")
        .expect("current projection");
    assert_eq!(
        current_projection.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Unknown
    );
    assert!(
        !current_projection
            .board_refs
            .iter()
            .any(|id| id == &replayed.id),
        "a stale Board origin remains Board-only without a projection ref"
    );
    let current_agent = current_projection
        .latest_agent_for_session("session-duplicate-assigned")
        .expect("current assigned Agent");
    assert_eq!(
        current_agent.current_focus.as_deref(),
        Some("Current blocked work")
    );
    assert_eq!(
        current_agent.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked
    );
    assert_eq!(current_agent.last_board_entry_id, None);
    assert_eq!(current_agent.last_board_entry_kind, None);
    assert_eq!(current_agent.updated_at, assigned_agent_at);
}

#[test]
fn app_runtime_active_work_projection_preserves_blocked_agent_board_state() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260504-1234");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = ActiveAgentSession {
        window_id: "tab-1::agent-1".to_string(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/20260504-1234".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: worktree.clone(),
        agent_project_root: worktree.display().to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session.clone());
    save_assigned_workspace_projection_for_test(&repo, &session).expect("save initial projection");
    let blocked = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Blocked,
        "Waiting for API credentials",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1")
    .with_origin_agent_id("codex")
    .with_origin_branch("work/20260504-1234");

    let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
    let projection = wait_for_active_work_projection(&mut runtime);

    assert_eq!(projection.status_category, "blocked");
    assert_eq!(projection.blocked_agents, 1);
    assert!(projection
        .agents
        .iter()
        .any(|agent| agent.session_id == "session-1"
            && agent.status_category == "blocked"
            && agent.last_board_entry_id.as_deref() == Some(blocked.id.as_str())));
    assert_eq!(projection.board_refs, vec![blocked.id.clone()]);
    assert_eq!(projection.next_action.as_deref(), Some("Resolve blocker"));
}

#[test]
fn app_runtime_active_work_projection_prioritizes_handoff_agents() {
    use gwt_core::workspace_projection::{
        WorkspaceAgentSummary, WorkspaceProjection, WorkspaceStatusCategory,
    };

    let mut projection = WorkspaceProjection::default_for_project("/repo");
    let now = chrono::Utc::now();
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-active".to_string(),
        window_id: Some("tab-1::agent-active".to_string()),
        agent_id: "codex".to_string(),
        display_name: "Alpha".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some("Implementing tests".to_string()),
        title_summary: None,
        worktree_path: None,
        branch: Some("work/20260504-1234".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: None,
        updated_at: now,
    });
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-handoff".to_string(),
        window_id: Some("tab-1::agent-handoff".to_string()),
        agent_id: "codex".to_string(),
        display_name: "Zulu".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some("Review visual state coverage".to_string()),
        title_summary: None,
        worktree_path: None,
        branch: Some("work/20260504-1234".to_string()),
        last_board_entry_id: Some("board-handoff".to_string()),
        last_board_entry_kind: Some(BoardEntryKind::Handoff),
        coordination_scope: Some("SPEC-2359 / workspace-ux".to_string()),
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: None,
        updated_at: now,
    });

    let view = active_work_projection_from_saved(projection);

    assert_eq!(view.agents[0].session_id, "session-handoff");
    assert_eq!(
        view.agents[0].last_board_entry_kind.as_deref(),
        Some("handoff")
    );
    assert_eq!(
        view.agents[0].coordination_scope.as_deref(),
        Some("SPEC-2359 / workspace-ux")
    );
}

#[test]
fn app_runtime_active_work_projection_recovers_blocked_agent_after_status_milestone() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260504-1234");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = ActiveAgentSession {
        window_id: "tab-1::agent-1".to_string(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/20260504-1234".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: worktree.clone(),
        agent_project_root: worktree.display().to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session.clone());
    save_assigned_workspace_projection_for_test(&repo, &session).expect("save initial projection");
    let blocked = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Blocked,
        "Waiting for API credentials",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");
    runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
    let status = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "API credentials configured",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");

    let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &status);
    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
    let projection = wait_for_active_work_projection(&mut runtime);

    assert_eq!(projection.status_category, "active");
    assert_eq!(projection.active_agents, 1);
    assert_eq!(projection.blocked_agents, 0);
    assert_eq!(projection.branch.as_deref(), Some("work/20260504-1234"));
}

#[test]
fn app_runtime_active_work_projection_keeps_blocked_agent_after_next_milestone() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260504-1234");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = ActiveAgentSession {
        window_id: "tab-1::agent-1".to_string(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/20260504-1234".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: worktree.clone(),
        agent_project_root: worktree.display().to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session.clone());
    save_assigned_workspace_projection_for_test(&repo, &session).expect("save initial projection");
    let blocked = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Blocked,
        "Waiting for API credentials",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");
    runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
    let next = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Next,
        "Try alternate credential source",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");

    let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &next);
    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
    let projection = wait_for_active_work_projection(&mut runtime);

    assert_eq!(projection.status_category, "blocked");
    assert_eq!(projection.active_agents, 0);
    assert_eq!(projection.blocked_agents, 1);
    assert_eq!(projection.status_text, "Waiting for API credentials");
    assert_eq!(
        projection.next_action.as_deref(),
        Some("Try alternate credential source")
    );
    assert_eq!(projection.branch.as_deref(), Some("work/20260504-1234"));
}

#[test]
fn app_runtime_agent_window_initial_title_uses_linked_issue_title() {
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
            2359,
            "SPEC: Workspace purpose titles",
            &["gwt-spec"],
            "Spec body",
            "2026-05-06T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260506-0736")
        .linked_issue_number(2359)
        .build();

    runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent_window.purpose_title.as_deref(),
        Some("SPEC: Workspace purpose titles")
    );
    assert_eq!(agent_window.title, "Codex");
}

// #3426: purpose-title lookup must use the canonical, workspace-home-aware
// issue-cache resolution (with the detached fallback) instead of the raw
// repo-root hash, matching title_sync.
#[test]
fn agent_launch_purpose_title_reads_detached_issue_cache_for_non_repo_root() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project_root = temp.path().join("plain-project");
    fs::create_dir_all(&project_root).expect("create project root");
    Cache::new(gwt::issue_cache::detached_issue_cache_root())
        .write_snapshot(&sample_issue_snapshot(
            3426,
            "fix(launch): stranded Active generation recovery",
            &[],
            "issue body",
            "2026-08-03T00:00:00Z",
        ))
        .expect("write detached issue cache");

    assert_eq!(
        super::super::workspace_views::agent_launch_purpose_title(
            &project_root,
            Some(3426),
            Some("work/issue-3426"),
            temp.path(),
        )
        .expect("healthy workspace state")
        .as_deref(),
        Some("fix(launch): stranded Active generation recovery"),
    );
}

#[test]
fn app_runtime_issue_monitor_enable_opens_single_settings_wizard_without_launch_settings() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3165,
            "SPEC: Issue auto-improve monitor",
            &["gwt-spec"],
            "Spec body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: true },
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("status resets optimistic enabled UI");
    assert!(
        !status.enabled,
        "Start without saved profile must return monitor status to stopped"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::IssueMonitorInbox { .. })),
        "Start without saved profile must not publish cached inbox yet"
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .is_some(),
        "Start without launch settings should open one settings wizard"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.issue_monitor_profile_save.as_ref())
        .is_some_and(|context| context.issue_number.is_none()));
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "settings wizard should be broadcast immediately"
    );
    assert!(
        runtime.window_details.is_empty(),
        "settings-required Start must not spawn an agent window"
    );
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .unwrap_or_default();
    assert!(
        !prefs.enabled,
        "Start without saved profile must not persist enabled state"
    );
}

#[test]
fn app_runtime_issue_monitor_enable_reports_missing_origin_detail() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _projection_timeout = ScopedEnvVar::set(
        "GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS",
        "10000",
    );

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_without_origin(&repo);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: true },
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    let error = status.last_error.as_deref().expect("origin error");
    assert!(
        error.starts_with("Git origin remote is not configured"),
        "unexpected error: {error}"
    );
    assert_ne!(error, "GitHub origin remote is unavailable");
}

#[test]
fn app_runtime_issue_monitor_enable_auto_without_manual_profile() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            launch_auto: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    reset_local_issue_monitor_remote_scan_count();
    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: true },
    );

    assert_eq!(
        local_issue_monitor_remote_scan_count(),
        0,
        "GUI control must not enter the remote scan path"
    );
    assert!(
        runtime.window_details.is_empty(),
        "GUI control must not launch"
    );
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("project state")
            .launch_wizard
            .is_none(),
        "auto settings must enable directly without a manual profile wizard"
    );
    let status = events.iter().find_map(|event| match &event.event {
        BackendEvent::IssueMonitorStatus { status } => Some(status),
        _ => None,
    });
    assert!(status.is_some_and(|status| status.enabled));
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(persisted.enabled);
    assert!(persisted.pending_effects.is_empty());
}

#[test]
fn app_runtime_issue_monitor_control_never_scans_or_claims_on_gui_thread() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    reset_local_issue_monitor_remote_scan_count();
    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: true },
    );

    assert_eq!(
        local_issue_monitor_remote_scan_count(),
        0,
        "GUI control must not enter the remote scan path"
    );
    assert!(
        runtime.window_details.is_empty(),
        "GUI control must not launch"
    );
    let status = events.iter().find_map(|event| match &event.event {
        BackendEvent::IssueMonitorStatus { status } => Some(status),
        _ => None,
    });
    assert!(status.is_some_and(|status| status.enabled));
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(persisted.enabled);
    assert!(persisted.pending_effects.is_empty());
}

#[test]
fn app_runtime_published_issue_monitor_control_has_one_authority_writer() {
    // A successful daemon publication must not replay the same authority
    // mutation through the GUI prefs path. The committed daemon projection is
    // delivered by BroadcastHub after its single disk transaction.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.issue_monitor_control_result_events(
        &runtime.test_context(),
        "client-1",
        Ok(()),
        "autonomous-mode",
        |monitor| {
            let _ = monitor.set_autonomous_mode_with_effect_revocation(false);
        },
    );

    assert!(
        events.is_empty(),
        "daemon will broadcast the committed status"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(persisted.autonomous_mode);
    assert_eq!(persisted.effect_authority_epoch, 7);
    assert!(persisted.pending_effects.is_empty());
}

#[test]
fn app_runtime_lifecycle_ack_never_replays_local_writer() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: None,
            }],
            failed_issues: vec![gwt::IssueMonitorFailedIssue {
                issue_number: 42,
                message: "prior failure".to_string(),
                window_id: Some("tab-1::stale-agent".to_string()),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed lifecycle prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "stale-agent",
        repo,
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let launched =
        runtime.issue_monitor_launch_succeeded_result_events(42, "tab-1::agent-1", Ok(()));
    let failed = runtime.issue_monitor_launch_failed_result_events(42, "launch failed", Ok(()));
    let closed = runtime.issue_monitor_window_closed_result_events("tab-1::agent-1", Ok(()));

    assert!(launched
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    assert!(
        !runtime.window_lookup.contains_key("tab-1::stale-agent"),
        "ACK closes the stale failed window captured before daemon commit"
    );
    assert!(failed.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorLaunchFailed { issue_number, .. } if *issue_number == 42
    )));
    assert!(closed.is_empty());
    assert_eq!(
        fs::read(&prefs_path).expect("reload prefs"),
        before,
        "ACKed lifecycle controls have exactly one daemon writer"
    );

    let rejected = runtime.issue_monitor_launch_failed_result_events(
        42,
        "must not be displayed as committed",
        Err(gwt::runtime_daemon_events::IssueMonitorControlPublishError::RecoveryBlocked),
    );
    assert!(rejected
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::IssueMonitorLaunchFailed { .. })));
    assert_eq!(
        fs::read(&prefs_path).expect("reload rejected prefs"),
        before
    );
}

#[test]
fn app_runtime_agent_failed_ack_runs_ui_finalize_without_a_local_write() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab-1::agent-1".to_string(),
            }],
            autonomous_records: vec![gwt::AutonomousIssueRecord {
                issue_number: 42,
                phase: gwt::AutonomousPhase::Implementing,
                active_launch_id: None,
                attempts: 1,
                non_agent_attempts: 0,
                acceptance_snapshot: None,
                retry_not_before: None,
                retry_hold_reason: None,
                retry_hold_provider: None,
                last_heartbeat: None,
                pr_number: None,
                reviewed_sha: None,
                review_passed: None,
                wait: None,
                needs_human_kind: None,
                steering: None,
                review_dispatch_hold: None,
                last_failure_message: None,
                delivering_since: None,
                review_attempts: None,
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed autonomous lifecycle prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1";
    runtime.pending_launch_feedback_contexts.insert(
        window_id.to_string(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: Some(repo),
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );

    let events = runtime.issue_monitor_agent_failed_result_events(
        window_id,
        "agent failed",
        Some(42),
        Ok(()),
    );

    assert!(!runtime
        .pending_launch_feedback_contexts
        .contains_key(window_id));
    assert!(!runtime.window_lookup.contains_key(window_id));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, issue_number, .. }
            if level == "error" && message == "agent failed" && *issue_number == Some(42)
    )));
    assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);
}

#[test]
fn app_runtime_agent_failed_ack_closes_default_mode_monitor_bootstrap_error_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: false,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab-1::agent-1".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed default-mode lifecycle prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1";
    runtime.pending_launch_feedback_contexts.insert(
        window_id.to_string(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: Some(repo),
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );

    let bootstrap_error =
        "Process exited with status: 1\naccount/read workspace routing discovery failed (-32603)";
    insert_test_pane_runtime(&mut runtime, window_id);
    let _ = runtime.issue_monitor_agent_failed_result_events(
        window_id,
        bootstrap_error,
        Some(42),
        Ok(()),
    );
    assert!(
        runtime.tracked_window_exists(window_id),
        "a Monitor failure ACK without a current PTY exit must retain the live pane"
    );
    runtime.runtimes.remove(window_id);
    let events = runtime.issue_monitor_agent_failed_result_events(
        window_id,
        bootstrap_error,
        Some(42),
        Ok(()),
    );

    assert!(!runtime
        .pending_launch_feedback_contexts
        .contains_key(window_id));
    assert!(
        !runtime.tracked_window_exists(window_id),
        "a failed Monitor bootstrap must close before the row can relaunch"
    );
    assert!(runtime.tabs[0].workspace.persisted().windows.is_empty());
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, issue_number, .. }
            if level == "error" && *issue_number == Some(42)
    )));
    assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);

    let mut fresh = pending_fresh_execution_fixture(temp.path(), "monitor-bootstrap-before-start");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fresh.repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed unbound Monitor prefs");
    fresh
        .runtime
        .pending_fresh_execution_launches
        .get_mut(&fresh.window_id)
        .expect("pending fresh execution")
        .launch_feedback_context = Some(LaunchFeedbackContext {
        client_id: "__issue_monitor__".to_string(),
        title: "Issue Monitor".to_string(),
        issue_monitor_issue_number: Some(fresh.owner.number),
        issue_monitor_delivery_id: None,
        issue_monitor_project_root: Some(fresh.repo.clone()),
        issue_monitor_session_mode: None,
        issue_monitor_autonomous_handoff: None,
        issue_monitor_autonomous_submit_started: false,
        issue_monitor_review_dispatch: false,
    });
    let events = fresh.runtime.issue_monitor_agent_failed_result_events(
        &fresh.window_id,
        bootstrap_error,
        None,
        Ok(()),
    );
    assert!(
        !fresh.runtime.tracked_window_exists(&fresh.window_id),
        "an unbound Monitor bootstrap failure must roll back before another launch"
    );
    assert_pending_fresh_execution_was_rolled_back(&fresh);
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, issue_number, .. }
            if level == "error" && *issue_number == Some(fresh.owner.number)
    )));

    let mut retained =
        pending_fresh_execution_fixture(temp.path(), "monitor-bootstrap-cleanup-refused");
    let mut feedback = issue_monitor_feedback(retained.owner.number);
    feedback.issue_monitor_project_root = Some(retained.repo.clone());
    feedback.issue_monitor_session_mode = Some(gwt_agent::SessionMode::Normal);
    retained
        .runtime
        .pending_fresh_execution_launches
        .get_mut(&retained.window_id)
        .expect("pending fresh execution")
        .launch_feedback_context = Some(feedback.clone());
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&retained.repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 2,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed spare Monitor capacity");
    replace_fresh_candidate_session_incarnation(
        &retained.runtime.sessions_dir,
        &retained.candidate_session_id,
    );
    retained
        .runtime
        .set_window_status("tab-1", "agent-1", WindowProcessStatus::Error);
    let _ = retained.runtime.issue_monitor_agent_failed_result_events(
        &retained.window_id,
        bootstrap_error,
        None,
        Ok(()),
    );
    assert!(retained.runtime.tracked_window_exists(&retained.window_id));
    assert!(retained
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&retained.window_id));
    retained.runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&retained.repo)
        .branch("work/issue-2359")
        .linked_issue_number(retained.owner.number)
        .build();
    // The RED path may create a sample canvas pane, but must never run a provider.
    config.command = retained
        .repo
        .join("missing-monitor-agent")
        .display()
        .to_string();
    for _ in 0..2 {
        let result = retained.runtime.spawn_agent_window_with_feedback(
            "tab-1",
            config.clone(),
            canvas_bounds(),
            None,
            feedback.clone(),
        );
        assert!(
            result.as_ref().is_err_and(|reason| reason.contains("pending")),
            "a retained Prepared Monitor launch must block a second pane despite spare capacity: {result:?}"
        );
        assert_eq!(
            retained.runtime.tabs[0].workspace.persisted().windows.len(),
            1
        );
    }
}

#[test]
fn app_runtime_provider_quota_fallback_persists_the_reported_provider() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let window_id = "tab-1::agent-1";
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            autonomous_mode: true,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: window_id.to_string(),
            }],
            // Issue #4366: one refusal alone no longer forms a hold, so the
            // reported provider is already held; the fallback must extend
            // *that* provider's hold to the reported reset, never the saved
            // profile's.
            provider_quota_holds: std::collections::BTreeMap::from([(
                "codex".to_string(),
                "2099-08-21T00:00:00Z".to_string(),
            )]),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed provider launch");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.provider_quota_holds.insert(
        window_id.to_string(),
        gwt::IssueMonitorFailure::ProviderUsageLimit {
            provider: "codex".to_string(),
            resets_at: Some("2099-08-22T04:00:00Z".to_string()),
            evidence: None,
        },
    );

    let _events = runtime.issue_monitor_agent_failed_result_events(
        window_id,
        "Codex usage limit reached",
        Some(42),
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "daemon not running".to_string(),
            ),
        ),
    );

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload quota prefs");
    assert_eq!(
        persisted
            .provider_quota_holds
            .get("codex")
            .map(String::as_str),
        Some("2099-08-22T04:00:00Z")
    );
    assert!(!persisted.provider_quota_holds.contains_key("claude"));
    let mut restored =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), persisted);
    assert_eq!(
        restored.status_view_at("2026-08-22T03:00:00Z").quota_hold,
        None,
        "the saved Claude profile must not project the Codex hold"
    );
    restored.set_gui_connected(true);
    restored.terminal_queue_push(&[43], "operator", "2026-07-28T00:00:00Z");
    restored.record_candidate(gwt::IssueMonitorIssue {
        number: 43,
        title: "Healthy provider candidate".to_string(),
        labels: Vec::new(),
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    });
    assert_eq!(
        restored
            .try_prepare_claim_effects_with_probe(
                "host/session",
                "2026-08-22T03:00:00Z",
                1,
                |_| Ok::<bool, std::convert::Infallible>(false),
            )
            .expect("infallible probe"),
        1
    );
}

#[test]
fn app_runtime_agent_failed_fallback_is_fail_closed_on_corrupt_prefs() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    fs::create_dir_all(prefs_path.parent().expect("prefs parent")).expect("create prefs parent");
    let corrupt = b"{\"enabled\":true";
    fs::write(&prefs_path, corrupt).expect("seed corrupt prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1";
    runtime.pending_launch_feedback_contexts.insert(
        window_id.to_string(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: Some(repo),
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );

    let events = runtime.issue_monitor_agent_failed_result_events(
        window_id,
        "agent failed",
        Some(42),
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "daemon not running".to_string(),
            ),
        ),
    );

    assert_eq!(fs::read(&prefs_path).expect("read raw prefs"), corrupt);
    assert!(fs::read_dir(prefs_path.parent().expect("prefs parent"))
        .expect("read prefs parent")
        .flatten()
        .all(|entry| !entry
            .file_name()
            .to_string_lossy()
            .starts_with("issue-monitor.json.corrupt-")));
    assert!(runtime
        .pending_launch_feedback_contexts
        .contains_key(window_id));
    assert!(runtime.window_lookup.contains_key(window_id));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains("local fallback control commit failed")
    )));
}

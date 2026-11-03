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
                worktree_path: Some(current_worktree.clone()),
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
    let event = current
        .events
        .iter()
        .find(|event| event.board_entry_id.as_deref() == Some(entry.id.as_str()))
        .expect("Board milestone event");
    let shard = gwt_core::paths::gwt_repo_local_work_event_shard_path(&current_worktree, &event.id);
    let published: gwt_core::workspace_projection::WorkEvent =
        serde_json::from_slice(&fs::read(shard).expect("read owning Work shard"))
            .expect("parse owning Work shard");
    let container = published.execution_container.expect("portable container");
    assert_eq!(container.worktree_path.as_deref(), Some(Path::new(".")));
    assert_eq!(container.branch.as_deref(), Some("feature/current"));
    assert!(
        !gwt_core::paths::gwt_repo_local_work_event_shard_path(&repo, &event.id).exists(),
        "the project root must not publish another Worktree's event"
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
    let worktree = repo.clone();
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
    let worktree = repo.clone();
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
    let worktree = repo.clone();
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

    spawn_owner_codex_fixture(
        &mut runtime,
        "tab-1",
        config,
        Some("SPEC: Workspace purpose titles"),
    );

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
        .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })));
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
                cancelled_claim_phase: None,
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
    use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let bootstrap_error =
        "Process exited with status: 1\naccount/read workspace routing discovery failed (-32603)";
    let (repo, owner, pane, mut runtime, mut observed) =
        super::managed_launch_tests::managed_monitor_exit_fixture(
            &temp.path().join("bound-bootstrap"),
            42,
            1,
            bootstrap_error,
        );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let window_id = runtime.runtimes.keys().next().unwrap().clone();
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let child_pid = pane.pty.process_id().unwrap();
    let bound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert!(
            !gwt::load_issue_monitor_prefs(&prefs_path)
                .unwrap()
                .autonomous_mode
        );
        let mut feedback = issue_monitor_feedback(42);
        feedback.issue_monitor_project_root = Some(repo.clone());
        runtime
            .pending_launch_feedback_contexts
            .insert(window_id.clone(), feedback.clone());
        owner_fixture_block_on(owner.wait_monitor_idle()).unwrap();
        let before = fs::read(&prefs_path).unwrap();
        runtime.issue_monitor_agent_failed_result_events(
            &window_id,
            bootstrap_error,
            Some(42),
            Ok(()),
        );
        assert_eq!(fs::read(&prefs_path).unwrap(), before);
        assert!(
            runtime.tracked_window_exists(&window_id),
            "a Monitor failure ACK without a current PTY exit must retain the live pane"
        );
        let live = owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).unwrap();
        assert_eq!(live.session.as_ref().unwrap().id, session_id);
        assert_eq!(live.pty.process_id(), Some(child_pid));
        assert!(live.pty.try_wait().unwrap().is_none());
        // The physical owner settles its real exit before the second read-only callback.
        super::managed_launch_tests::finish_managed_exit(
            &owner,
            &pane,
            &mut runtime,
            &mut observed,
        );
        owner_fixture_block_on(owner.wait_monitor_idle()).unwrap();
        let stopped = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
        )
        .unwrap();
        assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
        assert!(pane.pty.tree_exit_confirmed());
        assert!(owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_err());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let session_path = sessions.join(format!("{session_id}.toml"));
        let sidecar_path =
            gwt_agent::runtime_state_path_for_pid(&sessions, std::process::id(), &session_id);
        let registry_path = gwt::workspace_state_path(&repo);
        let before_session = fs::read(&session_path).unwrap();
        let before_sidecar = fs::read(&sidecar_path).unwrap();
        let before_registry = fs::read(&registry_path).unwrap();
        // Keep the real stopped runtime and its causal close receipt intact.
        // A separate test-owned namespace contributes current unknown evidence.
        let unknown_host = std::process::id().checked_add(1).unwrap();
        let unknown_path =
            gwt_agent::runtime_state_path_for_pid(&sessions, unknown_host, &session_id);
        assert!(!unknown_path.exists());
        fs::create_dir_all(unknown_path.parent().unwrap()).unwrap();
        fs::write(&unknown_path, b"unreadable foreign runtime proof").unwrap();
        let observations = gwt::session_inventory::observe_session(&stopped, &sessions);
        assert!(observations.sessions.is_empty());
        assert!(observations.uncertainties.iter().any(|uncertainty| {
            uncertainty.runtime_path == unknown_path
                && uncertainty.session_id.as_deref() == Some(session_id.as_str())
                && uncertainty.reason.starts_with("runtime_unreadable:")
        }));
        let refused = owner_fixture_block_on(owner.command(OwnerCommand::CloseStoppedSession {
            session_id: session_id.clone(),
            expected: Box::new(stopped.clone()),
        }));
        let unknown_refused = refused.is_err();
        let after_session = fs::read(&session_path).unwrap();
        let after_sidecar = fs::read(&sidecar_path).unwrap();
        let after_registry = fs::read(&registry_path).unwrap();
        // Remove only this fixture's uncertainty before retrying or asserting,
        // so a failed negative assertion cannot strand the owner in cleanup.
        fs::remove_file(&unknown_path).unwrap();
        // DiagnosticExit has already retired the physical runtime. Its saved
        // row is closed through the owner's exact stopped-Session CAS, before
        // the read-only GUI failure acknowledgement removes its observation.
        let cleanup = if unknown_refused {
            owner_fixture_block_on(owner.command(OwnerCommand::Close {
                id: pane.id.clone(),
                incarnation: pane.incarnation,
            }))
        } else {
            refused
        };
        if cleanup.is_err() {
            let sessions = gwt_core::paths::gwt_sessions_dir();
            let current = gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml")));
            let expected = serde_json::to_value(&stopped).unwrap();
            let current_value = current
                .as_ref()
                .ok()
                .map(|session| serde_json::to_value(session).unwrap());
            let changed_fields = expected
                .as_object()
                .unwrap()
                .keys()
                .filter(|field| {
                    current_value
                        .as_ref()
                        .and_then(|current| current.get(*field))
                        != expected.get(*field)
                })
                .cloned()
                .collect::<Vec<_>>();
            let identity = gwt_agent::SessionExecutionIdentity::from_session(&stopped)
                .ok()
                .flatten();
            let sidecar_path =
                gwt_agent::runtime_state_path_for_pid(&sessions, std::process::id(), &session_id);
            let sidecar_present =
                fs::symlink_metadata(&sidecar_path)
                    .map(|_| true)
                    .or_else(|error| {
                        if error.kind() == std::io::ErrorKind::NotFound {
                            Ok(false)
                        } else {
                            Err(error.kind())
                        }
                    });
            let runtime_proof = gwt_agent::SessionRuntimeState::load(&sidecar_path).map(|sidecar| {
                serde_json::json!({
                    "status":sidecar.status,
                    "incarnation_matches":sidecar.runtime_incarnation == Some(pane.incarnation),
                    "identity_matches":sidecar.execution_identity == identity,
                    "host_birth_matches":sidecar.host_started_at == gwt::process::host_process_start_time(std::process::id()),
                    "child_pid_matches":sidecar.child_pid == Some(child_pid),
                    "child_birth_present":sidecar.child_started_at.is_some_and(|birth| birth > 0),
                })
            }).map_err(|error| error.kind());
            let observations = current.as_ref().ok().map(|current| {
                let observations = gwt::session_inventory::observe_session(current, &sessions);
                (
                    observations.sessions.len(),
                    observations
                        .uncertainties
                        .iter()
                        .map(|uncertainty| {
                            uncertainty
                                .reason
                                .split(':')
                                .next()
                                .unwrap_or("")
                                .to_string()
                        })
                        .collect::<Vec<_>>(),
                )
            });
            eprintln!("stopped diagnostic cleanup refused: full_session_matches={}, changed_fields={changed_fields:?}, sidecar_present={sidecar_present:?}, runtime_proof={runtime_proof:?}, observations={observations:?}, tree_exit={}, pty_exit={}, owner_runtime_absent={}",
                current_value.as_ref() == Some(&expected), pane.pty.tree_exit_confirmed(),
                pane.pty.try_wait().is_ok_and(|exit| exit.is_some()),
                owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_err());
        }
        cleanup.expect("the physical owner acknowledges cleanup of the exact stopped diagnostic");
        assert!(unknown_refused,
            "a causal old DiagnosticClosed receipt cannot override current unknown runtime evidence");
        assert_eq!(
            after_session, before_session,
            "unknown evidence must retain the full stopped Session"
        );
        assert_eq!(
            after_sidecar, before_sidecar,
            "unknown evidence must retain the exact current-host terminal sidecar"
        );
        assert_eq!(
            after_registry, before_registry,
            "unknown evidence must retain the canonical failed row before exact retry"
        );
        runtime
            .pending_launch_feedback_contexts
            .insert(window_id.clone(), feedback);
        let (closer, closes) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = closer;
        let recorded = match &runtime.proxy {
            AppEventProxy::Stub(events) => events.clone(),
            _ => panic!("recorded actual owner close completion"),
        };
        let before_callback = fs::read(&prefs_path).unwrap();
        let events = runtime.issue_monitor_agent_failed_result_events(
            &window_id,
            bootstrap_error,
            Some(42),
            Ok(()),
        );
        assert_eq!(fs::read(&prefs_path).unwrap(), before_callback);
        drain_queued_blocking_tasks(&closes);
        apply_recorded_window_close_finalized(&mut runtime, &recorded);
        assert!(!runtime
            .pending_launch_feedback_contexts
            .contains_key(&window_id));
        assert!(
            !runtime.tracked_window_exists(&window_id),
            "a failed Monitor bootstrap must close before the row can relaunch"
        );
        assert!(runtime.tabs[0].workspace.persisted().windows.is_empty());
        assert!(
            gwt::load_workspace_state(&gwt::workspace_state_path(&repo))
                .unwrap()
                .windows
                .is_empty(),
            "the actual owner must clear the exact canonical failed row"
        );
        assert!(events.iter().any(|event| matches!(&event.event,
            BackendEvent::IssueMonitorToast { level, issue_number, .. }
                if level == "error" && *issue_number == Some(42))));
        assert!(pane.pty.tree_exit_confirmed());
        assert!(pane.pty.try_wait().unwrap().is_some());
    }));
    runtime.runtimes.clear();
    owner_fixture_block_on(owner.shutdown());
    bound.unwrap_or_else(|panic| std::panic::resume_unwind(panic));

    // Preserve the original unbound rollback and foreign-candidate refusal cases.
    for foreign_candidate in [false, true] {
        let case = if foreign_candidate {
            "monitor-bootstrap-cleanup-refused"
        } else {
            "monitor-bootstrap-before-start"
        };
        let case_root = temp.path().join(case);
        let _case_home = ScopedGwtHome::set(&case_root);
        let fresh = owner_monitor_fresh_fixture(&case_root, case, false, false);
        let mut observed = fresh.owner.subscribe();
        let pane = fresh
            .executor
            .block_on(
                fresh
                    .owner
                    .probe(&fresh.captured.id, fresh.captured.incarnation),
            )
            .unwrap();
        let original = pane.session.clone().unwrap();
        let feedback = fresh.pending.launch_feedback_context.clone().unwrap();
        let root = feedback.issue_monitor_project_root.clone().unwrap();
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&root);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let candidate_path = sessions.join(format!("{}.toml", fresh.pending.binding.session_id));
        let raw = pane.id.split_once("::").unwrap().1;
        let window_id = combined_window_id("tab-1", raw);
        let mut tab = sample_project_tab("tab-1", "Repo", root.clone(), ProjectKind::Git, &[]);
        tab.workspace = WindowRegistry::from_persisted(
            gwt::load_workspace_state(&gwt::workspace_state_path(&root)).unwrap(),
        );
        let mut runtime = sample_runtime(&case_root, vec![tab], Some("tab-1"));
        runtime.sessions_dir = sessions.clone();
        runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
        let context = runtime.test_context();
        let identity = fresh.owner.client().identity().unwrap().clone();
        let state = runtime.project_state_mut(&context).unwrap();
        state.project_owner_connection_id = 1;
        state.project_owner_identity = Some(identity.clone());
        let snapshot = fresh
            .executor
            .block_on(fresh.owner.command(OwnerCommand::Snapshot {
                id: pane.id.clone(),
                incarnation: pane.incarnation,
            }))
            .unwrap();
        runtime.install_remote_owner_projection(
            &context,
            &window_id,
            fresh.owner.client(),
            serde_json::from_value(snapshot).unwrap(),
            pane.session.clone(),
        );
        let verified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert!(pane.pending_fresh_execution.is_some());
            assert!(
                runtime.pending_fresh_execution_launches.is_empty(),
                "the GUI must not manufacture Prepared launch authority"
            );
            let replacement = if !foreign_candidate {
                reject_owner_fresh_readiness(&fresh, Some("bootstrap-readiness-refused".into()));
                while let Ok(event) = observed.try_recv() {
                    runtime.handle_project_owner_event(&context, 1, &identity, event);
                }
                assert_owner_fresh_launch_rolled_back(&fresh);
                None
            } else {
                replace_fresh_candidate_session_incarnation(
                    &sessions,
                    &fresh.pending.binding.session_id,
                );
                Some(fs::read(&candidate_path).unwrap())
            };
            runtime
                .pending_launch_feedback_contexts
                .insert(window_id.clone(), feedback.clone());
            fresh
                .executor
                .block_on(fresh.owner.wait_monitor_idle())
                .unwrap();
            let before_callback = fs::read(&prefs_path).unwrap();
            let events = runtime.issue_monitor_agent_failed_result_events(
                &window_id,
                bootstrap_error,
                None,
                Ok(()),
            );
            assert_eq!(fs::read(&prefs_path).unwrap(), before_callback);
            assert!(events.iter().any(|event| matches!(&event.event,
                BackendEvent::IssueMonitorToast { level, issue_number, .. }
                    if level == "error" && *issue_number == Some(fresh.authority.number))));
            if !foreign_candidate {
                assert!(
                    !runtime.tracked_window_exists(&window_id),
                    "an unbound Monitor bootstrap failure must roll back before another launch"
                );
                assert_owner_fresh_launch_rolled_back(&fresh);
                return;
            }
            assert!(runtime.tracked_window_exists(&window_id));
            let replacement = replacement.unwrap();
            assert_eq!(
                fs::read(&candidate_path).unwrap(),
                replacement,
                "the read-only failure callback cannot rewrite a foreign candidate"
            );
            let refused = fresh
                .executor
                .block_on(fresh.owner.command(OwnerCommand::Close {
                    id: pane.id.clone(),
                    incarnation: pane.incarnation,
                }));
            assert!(
                refused.is_err(),
                "a foreign candidate must refuse exact rollback: {refused:?}"
            );
            assert_eq!(fs::read(&candidate_path).unwrap(), replacement);
            assert!(pane.pty.tree_exit_confirmed());
            assert!(durable_launch_recovery_exists(
                &sessions,
                &fresh.pending.binding.session_id
            ));
            assert_eq!(
                gwt::cli::execution_state::continuation_attempt_for_operation(
                    &fresh.repo,
                    fresh.authority,
                    &fresh.pending.operation_id,
                )
                .unwrap()
                .unwrap()
                .status,
                gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
            );
            let retained = fresh
                .executor
                .block_on(fresh.owner.command(OwnerCommand::ListPanes))
                .unwrap();
            assert_eq!(retained["reconciliation"].as_array().unwrap().len(), 1);
            assert_eq!(
                retained["canonical_rows"]["windows"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert!(
                owner_monitor_read(&root, fresh.authority.number).effective_max_active_agents()
                    >= 2,
                "the retained Prepared candidate must leave spare capacity"
            );
            let predecessor =
                gwt_agent::Session::load(&sessions.join(format!("predecessor-{case}.toml")))
                    .unwrap();
            let predecessor_identity =
                gwt_agent::SessionExecutionIdentity::from_session(&predecessor)
                    .unwrap()
                    .unwrap();
            assert_eq!(
                predecessor_identity.execution_binding.identity,
                fresh.predecessor_binding
            );
            for index in 0..2 {
                let roster = gwt::load_issue_monitor_prefs(&prefs_path)
                    .unwrap()
                    .agent_roster
                    .unwrap();
                assert!(roster.members.iter().any(|member| member.enabled
                    && member.assignment.is_none() && member.agent_id.as_deref() == Some("fixture")),
                    "a real idle fixture-tool member must be available before testing the Prepared refusal");
                let choice = gwt::project_runtime::launch_payload::ManualSuccessorChoice {
                    operation_id: format!("second-monitor-{index}"),
                    expected_binding: fresh.predecessor_binding.clone(),
                    expected_predecessor: Some(Box::new(predecessor_identity.clone())),
                    expected_runtime: None,
                    predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor::Blocked,
                };
                let mut command =
                    owner_fresh_agent_command(&fresh.owner, &fresh.repo, choice, None);
                if let OwnerCommand::LaunchAgent {
                    config, metadata, ..
                } = &mut command
                {
                    config.branch = Some(original.branch.clone());
                    config.base_branch = Some("origin/develop".into());
                    config.linked_issue_number = Some(fresh.authority.number);
                    config.explicit_follow_up = true;
                    config.launch_route = gwt_agent::LaunchRoute::Autonomous;
                    metadata.launch_feedback_context = Some(feedback.clone());
                }
                let result = fresh.executor.block_on(fresh.owner.command(command));
                assert!(result.as_ref().is_err_and(|reason| reason.contains(
                    "manual successor refuses while a Prepared successor or takeover targets the current generation")),
                    "a retained Prepared Monitor launch must block a second pane despite spare capacity: {result:?}");
                let inventory = fresh
                    .executor
                    .block_on(fresh.owner.command(OwnerCommand::ListPanes))
                    .unwrap();
                assert!(inventory["panes"].as_array().unwrap().is_empty());
                assert!(inventory["pending"].as_array().unwrap().is_empty());
                assert_eq!(inventory["reconciliation"], retained["reconciliation"]);
                assert_eq!(inventory["canonical_rows"], retained["canonical_rows"]);
                assert_eq!(fs::read(&candidate_path).unwrap(), replacement);
                assert!(durable_launch_recovery_exists(
                    &sessions,
                    &fresh.pending.binding.session_id
                ));
                assert_eq!(
                    gwt::cli::execution_state::continuation_attempt_for_operation(
                        &fresh.repo,
                        fresh.authority,
                        &fresh.pending.operation_id,
                    )
                    .unwrap()
                    .unwrap()
                    .status,
                    gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
                );
                assert_eq!(
                    gwt::cli::execution_state::current_execution_binding(
                        &fresh.repo,
                        fresh.authority
                    )
                    .unwrap(),
                    Some(fresh.predecessor_binding.clone())
                );
                assert_eq!(runtime.tabs[0].workspace.persisted().windows.len(), 1);
            }
            assert!(
                !std::iter::from_fn(|| observed.try_recv().ok()).any(
                    |event| matches!(event, OwnerEvent::AgentStarted { id, .. } if id != pane.id)
                ),
                "neither refused operation may start another physical Agent"
            );
        }));
        if foreign_candidate && candidate_path.exists() {
            // Restore only this fixture's foreign incarnation before exact normal teardown.
            original.save(&sessions).unwrap();
            fresh
                .executor
                .block_on(fresh.owner.command(OwnerCommand::Close {
                    id: pane.id.clone(),
                    incarnation: pane.incarnation,
                }))
                .expect("drain the exact retained Prepared candidate after restoring its snapshot");
        }
        runtime.runtimes.clear();
        fresh.shutdown();
        verified.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
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
    let mut restored = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), persisted);
    assert_eq!(
        restored.status_view_at("2026-08-22T03:00:00Z").quota_hold,
        None,
        "the saved Claude profile must not project the Codex hold"
    );
    restored.set_gui_connected(true);
    restored.terminal_queue_push(&[43], "operator", "2026-07-28T00:00:00Z");
    restored.record_candidate(gwt::IssueMonitorIssue {
        author_login: None,
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

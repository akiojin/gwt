use super::*;

#[test]
fn app_runtime_load_board_replies_with_repo_scoped_snapshot() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Need review",
            Some("running".to_string()),
            None,
            vec!["coordination".to_string()],
            vec!["2018".to_string()],
        ),
    )
    .expect("seed board snapshot");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadBoard {
            id: window_id.clone(),
            all: false,
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardEntries { id, entries, .. },
            ..
        }] if client_id == "client-1"
            && id == &window_id
            && entries.len() == 1
            && entries[0].body == "Need review"
    ));
}

#[test]
fn app_runtime_load_board_defaults_to_current_workspace_audience() {
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
    projection.id = "workspace-current".to_string();
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "session-current".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Board audience".to_string()),
            title_summary: Some("Board audience".to_string()),
            worktree_path: Some(repo.clone()),
            branch: Some("work/board-audience".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: Some("workspace-current".to_string()),
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "broadcast entry",
            None,
            None,
            vec![],
            vec![],
        ),
    )
    .expect("seed broadcast");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "current workspace entry",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["workspace-current"]),
    )
    .expect("seed current");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "other workspace entry",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["workspace-other"]),
    )
    .expect("seed other");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadBoard {
            id: window_id.clone(),
            all: false,
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            event: BackendEvent::BoardEntries { entries, .. },
            ..
        }] if entries.iter().map(|entry| entry.body.as_str()).collect::<Vec<_>>()
            == vec!["broadcast entry", "current workspace entry"]
    ));
}

#[test]
fn app_runtime_load_board_history_replies_with_older_page() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    for idx in 0..4 {
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            format!("entry-{idx}"),
            None,
            None,
            vec![],
            vec![],
        );
        entry.id = format!("entry-{idx}");
        entry.created_at = chrono::Utc::now() + chrono::Duration::seconds(idx);
        entry.updated_at = entry.created_at;
        post_entry(&repo, entry).expect("seed board entry");
    }
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadBoardHistory {
            id: window_id.clone(),
            before_entry_id: Some("entry-3".to_string()),
            limit: 2,
            all: false,
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardHistoryPage {
                id,
                entries,
                has_more_before,
            },
            ..
        }] if client_id == "client-1"
            && id == &window_id
            && entries.iter().map(|entry| entry.body.as_str()).collect::<Vec<_>>() == vec!["entry-1", "entry-2"]
            && *has_more_before
    ));
}

#[test]
fn app_runtime_open_board_origin_agent_focuses_live_origin_session_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let board_raw_id = tab
        .workspace
        .add_window(WindowPreset::Board, canvas_bounds())
        .id;
    let agent_raw_id = tab
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds())
        .id;
    let board_window_id = combined_window_id("tab-1", &board_raw_id);
    let agent_window_id = combined_window_id("tab-1", &agent_raw_id);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        agent_window_id.clone(),
        ActiveAgentSession {
            window_id: agent_window_id.clone(),
            session_id: "session-origin".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/board-origin".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenBoardOriginAgent {
            id: board_window_id,
            origin_session_id: "session-origin".to_string(),
            bounds: Some(canvas_bounds()),
        },
    );

    let workspace = runtime.tab("tab-1").expect("tab").workspace.persisted();
    let focused = workspace
        .windows
        .iter()
        .max_by_key(|window| window.z_index)
        .expect("focused window");
    assert_eq!(focused.id, agent_raw_id);
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            event: BackendEvent::WindowCanvasState { .. },
            ..
        }
    )));
}

#[test]
fn board_origin_agent_resume_config_uses_exact_saved_session() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session =
        gwt_agent::Session::new(&repo, "work/board-origin", gwt_agent::AgentId::Codex);
    session.id = "session-origin".to_string();
    session.agent_session_id = Some("codex-resume-123".to_string());
    session.model = Some("gpt-5.5".to_string());
    session.reasoning_level = Some("high".to_string());
    session.tool_version = Some("0.116.0".to_string());
    session.tool_version_selector = Some("latest".to_string());
    session.tool_runtime_provenance = Some(gwt_agent::ToolRuntimeProvenance {
        schema_version: gwt_agent::ToolRuntimeProvenance::CURRENT_SCHEMA_VERSION,
        official_package: "@openai/codex".to_string(),
        requested_selector: "latest".to_string(),
        resolved_exact_version: "0.116.0".to_string(),
        runner_kind: gwt_agent::ToolRuntimeRunnerKind::Npx,
        resolution_reason: gwt_agent::ToolRuntimeResolutionReason::RequestedSelector,
    });
    session.skip_permissions = true;
    session.codex_fast_mode = true;
    session.save(&runtime.sessions_dir).expect("save session");

    let config = runtime
        .board_origin_agent_resume_config("session-origin")
        .expect("resume config");

    assert_eq!(config.command, "codex");
    assert_eq!(config.branch.as_deref(), Some("work/board-origin"));
    assert_eq!(config.working_dir.as_deref(), Some(repo.as_path()));
    assert_eq!(
        config.resume_session_id.as_deref(),
        Some("codex-resume-123")
    );
    assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    assert_eq!(config.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(config.reasoning_level.as_deref(), Some("high"));
    assert!(config.skip_permissions);
    assert!(config.codex_fast_mode);
}

#[test]
fn board_origin_agent_resume_config_supports_builtin_agent_descriptors() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let runtime = sample_runtime(temp.path(), Vec::new(), None);

    for agent_id in [gwt_agent::AgentId::OpenClaw, gwt_agent::AgentId::Hermes] {
        let session_id = format!("session-{}", agent_id.command());
        let resume_id = format!("resume-{}", agent_id.command());
        let mut session = gwt_agent::Session::new(
            &repo,
            format!("work/{}", agent_id.command()),
            agent_id.clone(),
        );
        session.id = session_id.clone();
        session.agent_session_id = Some(resume_id.clone());
        session.save(&runtime.sessions_dir).expect("save session");

        let config = runtime
            .board_origin_agent_resume_config(&session_id)
            .expect("resume config");

        assert_eq!(config.agent_id, agent_id);
        assert_eq!(
            config.resume_session_id.as_deref(),
            Some(resume_id.as_str())
        );
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    }
}

#[test]
fn app_runtime_open_board_origin_agent_rejects_missing_exact_resume_session() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let board_window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenBoardOriginAgent {
            id: board_window_id.clone(),
            origin_session_id: "missing-session".to_string(),
            bounds: Some(canvas_bounds()),
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardError { id, message },
            ..
        }] if client_id == "client-1"
            && id == &board_window_id
            && message.contains("missing-session")
    ));
}

#[test]
fn app_runtime_load_knowledge_bridge_replies_with_cache_backed_issue_and_spec_views() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let cache = Cache::new(issue_cache_root(&repo));
    cache
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue bridge",
            &["bug"],
            "Issue body",
            "2026-04-20T10:00:00Z",
        ))
        .expect("write issue snapshot");
    cache
        .write_snapshot(&sample_issue_snapshot(
            1930,
            "SPEC-1930: Cache-backed SPEC bridge",
            &["gwt-spec", "phase/implementation"],
            concat!(
                "<!-- gwt-spec id=1930 version=1 -->\n",
                "<!-- sections:\n",
                "spec=body\n",
                "tasks=body\n",
                "-->\n\n",
                "<!-- artifact:spec BEGIN -->\n",
                "# SPEC bridge\n",
                "## Summary\n",
                "Cache-backed issue view\n",
                "<!-- artifact:spec END -->\n\n",
                "<!-- artifact:tasks BEGIN -->\n",
                "- [x] T-001\n",
                "<!-- artifact:tasks END -->\n"
            ),
            "2026-04-20T09:00:00Z",
        ))
        .expect("write spec snapshot");
    write_issue_link_store(
        &repo,
        HashMap::from([("feature/issue-bridge".to_string(), 42)]),
    );

    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "issue-1",
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    ));
    persisted.windows.push(sample_window(
        "spec-1",
        WindowPreset::Spec,
        WindowProcessStatus::Ready,
    ));
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo,
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let issue_window_id = combined_window_id("tab-1", "issue-1");
    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &issue_window_id,
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(42),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let issue_events = wait_for_knowledge_view_dispatch(&recorded_events, &issue_window_id);
    assert_eq!(issue_events.len(), 2);
    assert!(matches!(
        &issue_events[0].event,
        BackendEvent::KnowledgeEntries {
            knowledge_kind,
            entries,
            selected_number,
            refresh_enabled,
            ..
        } if *knowledge_kind == gwt::KnowledgeKind::Issue
            && entries.len() == 2
            && entries.iter().any(|entry| entry.number == 42
                && entry.linked_branch_count == 1
                && !entry.is_spec)
            && entries.iter().any(|entry| entry.number == 1930 && entry.is_spec)
            && *selected_number == Some(42)
            && *refresh_enabled
    ));
    assert!(matches!(
        &issue_events[1].event,
        BackendEvent::KnowledgeDetail { detail, .. }
            if detail.launch_issue_number == Some(42)
                && detail.sections.iter().any(|section| section.title == "Linked branches"
                    && section.body.contains("feature/issue-bridge"))
    ));

    let spec_window_id = combined_window_id("tab-1", "spec-1");
    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &spec_window_id,
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(1930),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let spec_events = wait_for_knowledge_view_dispatch(&recorded_events, &spec_window_id);
    assert_eq!(spec_events.len(), 2);
    assert!(matches!(
        &spec_events[0].event,
        BackendEvent::KnowledgeEntries {
            knowledge_kind,
            entries,
            selected_number,
            refresh_enabled,
            ..
        } if *knowledge_kind == gwt::KnowledgeKind::Issue
            && entries.len() == 2
            && entries.iter().any(|entry| entry.number == 1930 && entry.is_spec)
            && *selected_number == Some(1930)
            && *refresh_enabled
    ));
    assert!(matches!(
        &spec_events[1].event,
        BackendEvent::KnowledgeDetail { detail, .. }
            if detail.sections.iter().any(|section| section.title == "spec"
                && section.body.contains("Cache-backed issue view"))
    ));
}

#[test]
fn app_runtime_knowledge_load_joins_latest_project_monitor_snapshot() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let cache = Cache::new(issue_cache_root(&repo));
    for (number, title) in [
        (42, "First queued Issue"),
        (43, "Second queued Issue"),
        (44, "Held Issue"),
        (45, "Unmonitored Issue"),
    ] {
        cache
            .write_snapshot(&sample_issue_snapshot(
                number,
                title,
                &["bug"],
                "Issue body",
                "2026-08-10T00:00:00Z",
            ))
            .expect("write issue snapshot");
    }

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let mut held = pm_wake_inbox_item(44, gwt::MonitorInboxState::HoldExcluded);
    held.exclusion_reason = Some("Excluded by label: hold".to_string());
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
    monitor.terminal_queue_push(&[42, 43], "operator", "2026-07-28T00:00:00Z");
    monitor.inbox = vec![
        pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued),
        pm_wake_inbox_item(99, gwt::MonitorInboxState::Launching),
        pm_wake_inbox_item(43, gwt::MonitorInboxState::Queued),
        held,
    ];
    let _ = runtime.issue_monitor_snapshot_events_for(None, Some(&repo), monitor);

    let window_id = combined_window_id("tab-1", "issue-1");
    assert!(runtime
        .load_knowledge_bridge_events(
            "client-1",
            KnowledgeLoadRequest {
                id: &window_id,
                kind: gwt::KnowledgeKind::Issue,
                request_id: None,
                selected_number: None,
                refresh: false,
            },
        )
        .is_empty());
    let events = wait_for_knowledge_view_dispatch(&recorded_events, &window_id);
    let entries = events
        .iter()
        .find_map(|outbound| match &outbound.event {
            BackendEvent::KnowledgeEntries { entries, .. } => Some(entries),
            _ => None,
        })
        .expect("knowledge entries");
    let entry = |number| {
        entries
            .iter()
            .find(|entry| entry.number == number)
            .unwrap_or_else(|| panic!("missing Issue #{number}"))
    };

    assert_eq!(
        (
            entry(42).monitor_state,
            entry(42).queue_position,
            entry(42).exclusion_reason.as_deref(),
        ),
        (Some(gwt::MonitorInboxState::Queued), Some(1), None),
    );
    assert_eq!(
        (entry(43).monitor_state, entry(43).queue_position),
        (Some(gwt::MonitorInboxState::Queued), Some(2)),
    );
    assert_eq!(
        (
            entry(44).monitor_state,
            entry(44).queue_position,
            entry(44).exclusion_reason.as_deref(),
        ),
        (
            Some(gwt::MonitorInboxState::HoldExcluded),
            None,
            Some("Excluded by label: hold"),
        ),
    );
    assert_eq!(
        (
            entry(45).monitor_state,
            entry(45).queue_position,
            entry(45).exclusion_reason.as_deref(),
        ),
        (None, None, None),
    );
}

/// Issue #3297: the cache-backed knowledge load must not run on the GUI
/// event loop. On slow machines the synchronous read blocked the loop past
/// the frontend's 5s recovery timer, the late reply was discarded, and the
/// retry escalated into a minutes-long forced sync. The load replies through
/// the blocking-task proxy instead, like the semantic search path.
#[test]
fn app_runtime_load_knowledge_bridge_replies_off_the_gui_event_loop() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let cache = Cache::new(issue_cache_root(&repo));
    cache
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue bridge",
            &["bug"],
            "Issue body",
            "2026-04-20T10:00:00Z",
        ))
        .expect("write issue snapshot");

    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "issue-1",
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    ));
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo,
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &window_id,
            kind: gwt::KnowledgeKind::Issue,
            request_id: Some(7),
            selected_number: Some(42),
            refresh: false,
        },
    );

    assert!(
        immediate.is_empty(),
        "cache-backed knowledge load must not reply on the frontend event loop"
    );
    wait_for_recorded_event("knowledge entries dispatch", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.target,
                            DispatchTarget::Client(client_id) if client_id == "client-1"
                        ) && matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeEntries {
                                id,
                                request_id,
                                entries,
                                selected_number,
                                ..
                            } if id == &window_id
                                && *request_id == Some(7)
                                && entries.iter().any(|entry| entry.number == 42)
                                && *selected_number == Some(42)
                        )
                    })
            )
        })
    });
    wait_for_recorded_event("knowledge detail dispatch", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| matches!(
                        &outbound.event,
                        BackendEvent::KnowledgeDetail { id, .. } if id == &window_id
                    ))
            )
        })
    });
}

#[test]
fn app_runtime_load_knowledge_bridge_projects_related_issue_work_sessions() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3096");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3096,
            "Fix Launch Agent trace",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let mut session =
        gwt_agent::Session::new(&worktree, "work/issue-3096", gwt_agent::AgentId::Codex);
    session.id = "session-issue-3096".to_string();
    session.agent_session_id = Some("conv-issue-3096".to_string());
    session.display_name = "Codex".to_string();
    session.linked_issue_number = Some(3096);
    session.save(&runtime.sessions_dir).expect("save session");

    let now = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-3096"),
        Some(&worktree),
    )
    .expect("work id");
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    event.title = Some("Fix Launch Agent trace".to_string());
    event.owner = Some("Issue #3096".to_string());
    event.agent_session_id = Some("session-issue-3096".to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3096".to_string()),
            worktree_path: Some(worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
        .expect("record work event");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3096),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let entries = match &events[0].event {
        BackendEvent::KnowledgeEntries { entries, .. } => entries,
        other => panic!("unexpected entries event: {other:?}"),
    };
    assert_eq!(entries[0].number, 3096);
    assert_eq!(entries[0].related_work_count, 1);
    assert_eq!(entries[0].related_session_count, 1);

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    let work = &detail.related_works[0];
    assert_eq!(work.title, "Fix Launch Agent trace");
    assert_eq!(work.branch.as_deref(), Some("work/issue-3096"));
    assert_eq!(work.agents[0].session_id, "session-issue-3096");
    assert_eq!(
        work.agents[0].sessions[0].agent_session_id,
        "conv-issue-3096"
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_marks_session_only_stopped_related_session_past() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3133");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3133,
            "Resume historical Launch Agent session",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let stopped_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let mut session =
        gwt_agent::Session::new(&worktree, "work/issue-3133", gwt_agent::AgentId::Codex);
    session.id = "session-issue-3133-stopped".to_string();
    session.agent_session_id = Some("conv-issue-3133-stopped".to_string());
    session.status = gwt_agent::AgentStatus::Stopped;
    session.linked_issue_number = Some(3133);
    session.created_at = stopped_at;
    session.updated_at = stopped_at;
    session.last_activity_at = stopped_at;
    session.save(&runtime.sessions_dir).expect("save session");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3133),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    assert_eq!(detail.related_works[0].status_category, "idle");
    assert_eq!(detail.related_works[0].agents[0].sessions.len(), 1);
    assert!(
        !detail.related_works[0].agents[0].sessions[0].is_active,
        "session-only stopped related sessions must render as Past, not Current"
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_dedupes_related_issue_sessions_by_conversation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3133");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3133,
            "Resume Launch Agent session",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let conversation_id = "conv-issue-3133";
    let old_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 0, 0).unwrap();
    let new_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();

    let mut stale_session =
        gwt_agent::Session::new(&worktree, "work/issue-3133", gwt_agent::AgentId::Codex);
    stale_session.id = "session-issue-3133-stale".to_string();
    stale_session.agent_session_id = Some(conversation_id.to_string());
    stale_session.status = gwt_agent::AgentStatus::Stopped;
    stale_session.linked_issue_number = Some(3133);
    stale_session.created_at = old_at;
    stale_session.updated_at = old_at;
    stale_session.last_activity_at = old_at;
    stale_session
        .save(&runtime.sessions_dir)
        .expect("save stale session");

    let mut current_session =
        gwt_agent::Session::new(&worktree, "work/issue-3133", gwt_agent::AgentId::Codex);
    current_session.id = "session-issue-3133-current".to_string();
    current_session.agent_session_id = Some(conversation_id.to_string());
    current_session.status = gwt_agent::AgentStatus::Running;
    current_session.linked_issue_number = Some(3133);
    current_session.created_at = new_at;
    current_session.updated_at = new_at;
    current_session.last_activity_at = new_at;
    current_session
        .save(&runtime.sessions_dir)
        .expect("save current session");

    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-3133"),
        Some(&worktree),
    )
    .expect("work id");
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        new_at,
    );
    event.title = Some("Resume Launch Agent session".to_string());
    event.owner = Some("Issue #3133".to_string());
    event.agent_session_id = Some("session-issue-3133-current".to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3133".to_string()),
            worktree_path: Some(worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
        .expect("record work event");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3133),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let entries = match &events[0].event {
        BackendEvent::KnowledgeEntries { entries, .. } => entries,
        other => panic!("unexpected entries event: {other:?}"),
    };
    assert_eq!(entries[0].number, 3133);
    assert_eq!(entries[0].related_session_count, 1);

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    assert_eq!(detail.related_works[0].agents.len(), 1);
    assert_eq!(detail.related_works[0].agents[0].sessions.len(), 1);
    assert_eq!(
        detail.related_works[0].agents[0].session_id,
        "session-issue-3133-current"
    );
    assert_eq!(
        detail.related_works[0].agents[0].sessions[0].agent_session_id,
        conversation_id
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_collapses_related_issue_session_actions_to_live_latest() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3133");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3133,
            "Resume Launch Agent session",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let stale_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 0, 0).unwrap();
    let past_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 1, 0).unwrap();
    let live_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let empty_agent_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 6, 0).unwrap();

    for (id, conversation_id, status, timestamp) in [
        (
            "session-issue-3133-stale-live",
            "conv-issue-3133-live",
            gwt_agent::AgentStatus::Stopped,
            stale_at,
        ),
        (
            "session-issue-3133-past",
            "conv-issue-3133-past",
            gwt_agent::AgentStatus::Stopped,
            past_at,
        ),
        (
            "session-issue-3133-current-live",
            "conv-issue-3133-live",
            gwt_agent::AgentStatus::Running,
            live_at,
        ),
    ] {
        let mut session =
            gwt_agent::Session::new(&worktree, "work/issue-3133", gwt_agent::AgentId::Codex);
        session.id = id.to_string();
        session.agent_session_id = Some(conversation_id.to_string());
        session.status = status;
        session.linked_issue_number = Some(3133);
        session.created_at = timestamp;
        session.updated_at = timestamp;
        session.last_activity_at = timestamp;
        session.save(&runtime.sessions_dir).expect("save session");
    }

    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-3133"),
        Some(&worktree),
    )
    .expect("work id");
    for (session_id, timestamp) in [
        ("session-issue-3133-stale-live", stale_at),
        ("session-issue-3133-past", past_at),
        ("session-issue-3133-current-live", live_at),
    ] {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            work_id.clone(),
            timestamp,
        );
        event.title = Some("Resume Launch Agent session".to_string());
        event.owner = Some("Issue #3133".to_string());
        event.agent_session_id = Some(session_id.to_string());
        event.agent_id = Some("codex".to_string());
        event.display_name = Some("Codex".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/issue-3133".to_string()),
                worktree_path: Some(worktree.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
            .expect("record work event");
    }
    let mut empty_agent_event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id.clone(),
        empty_agent_at,
    );
    empty_agent_event.title = Some("Resume Launch Agent session".to_string());
    empty_agent_event.owner = Some("Issue #3133".to_string());
    empty_agent_event.agent_session_id = Some("session-issue-3133-empty".to_string());
    empty_agent_event.agent_id = Some("codex".to_string());
    empty_agent_event.display_name = Some("Codex".to_string());
    empty_agent_event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3133".to_string()),
            worktree_path: Some(worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, empty_agent_event)
        .expect("record empty agent work event");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3133),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let entries = match &events[0].event {
        BackendEvent::KnowledgeEntries { entries, .. } => entries,
        other => panic!("unexpected entries event: {other:?}"),
    };
    assert_eq!(entries[0].number, 3133);
    assert_eq!(entries[0].related_session_count, 1);

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    assert_eq!(detail.related_works[0].agents.len(), 1);
    assert_eq!(
        detail.related_works[0].agents[0].session_id,
        "session-issue-3133-current-live"
    );
    assert_eq!(detail.related_works[0].agents[0].sessions.len(), 1);
    assert_eq!(
        detail.related_works[0].agents[0].sessions[0].agent_session_id,
        "conv-issue-3133-live"
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_ignores_ambiguous_branch_only_related_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let develop_worktree = temp.path().join("repo-develop");
    let issue_worktree = temp.path().join("repo-work-issue-3133");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&develop_worktree).expect("create develop worktree");
    fs::create_dir_all(&issue_worktree).expect("create issue worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3133,
            "Resume Launch Agent session",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");
    write_issue_link_store(
        &repo,
        HashMap::from([("work/issue-3133".to_string(), 3133)]),
    );

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let live_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let mut session = gwt_agent::Session::new(
        &issue_worktree,
        "work/issue-3133",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-issue-3133-current".to_string();
    session.agent_session_id = Some("conv-issue-3133-current".to_string());
    session.status = gwt_agent::AgentStatus::Running;
    session.linked_issue_number = Some(3133);
    session.created_at = live_at;
    session.updated_at = live_at;
    session.last_activity_at = live_at;
    session.save(&runtime.sessions_dir).expect("save session");

    let issue_work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-3133"),
        Some(&issue_worktree),
    )
    .expect("issue work id");
    let mut issue_event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        issue_work_id,
        live_at,
    );
    issue_event.title = Some("Issue #3133 visual verification".to_string());
    issue_event.owner = Some("Issue #3133".to_string());
    issue_event.agent_session_id = Some("session-issue-3133-current".to_string());
    issue_event.agent_id = Some("codex".to_string());
    issue_event.display_name = Some("Codex".to_string());
    issue_event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3133".to_string()),
            worktree_path: Some(issue_worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, issue_event)
        .expect("record issue work event");

    let ambiguous_id = "legacy-ambiguous-branch-only";
    for (branch, worktree, at) in [
        (
            "develop",
            develop_worktree.as_path(),
            Utc.with_ymd_and_hms(2026, 6, 20, 9, 6, 0).unwrap(),
        ),
        (
            "work/issue-3133",
            issue_worktree.as_path(),
            Utc.with_ymd_and_hms(2026, 6, 20, 9, 7, 0).unwrap(),
        ),
    ] {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            ambiguous_id,
            at,
        );
        event.title = Some("Work progress summary detail".to_string());
        event.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Unknown);
        event.agent_session_id = Some("missing-legacy-session".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(branch.to_string()),
                worktree_path: Some(worktree.to_path_buf()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
            .expect("record ambiguous work event");
    }

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3133),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let entries = match &events[0].event {
        BackendEvent::KnowledgeEntries { entries, .. } => entries,
        other => panic!("unexpected entries event: {other:?}"),
    };
    assert_eq!(entries[0].number, 3133);
    assert_eq!(entries[0].related_work_count, 1);
    assert_eq!(entries[0].related_session_count, 1);

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    assert_eq!(
        detail.related_works[0].title,
        "Issue #3133 visual verification"
    );
    assert!(
        detail
            .related_works
            .iter()
            .all(|work| work.title != "Work progress summary detail"),
        "ambiguous branch-only Work must not appear in Issue related work"
    );
}

#[test]
fn app_runtime_knowledge_search_errors_for_wrong_surface() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let events = runtime.search_knowledge_bridge_events(
        "client-1",
        KnowledgeSearchRequest {
            id: &window_id,
            kind: gwt::KnowledgeKind::Issue,
            query: "semantic query",
            request_id: 9,
            selected_number: None,
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::KnowledgeError {
                knowledge_kind,
                message,
                ..
            },
            knowledge_wire_metadata: Some(super::super::KnowledgeWireMetadata::NonSemanticError),
            ..
        }] if client_id == "client-1"
            && *knowledge_kind == gwt::KnowledgeKind::Issue
            && message == "Window is not a knowledge bridge"
    ));
}

#[cfg(unix)]
#[test]
fn app_runtime_knowledge_search_replies_through_async_dispatch() {
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    write_fake_project_index_runtime(temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let cache = Cache::new(issue_cache_root(&repo));
    cache
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Async semantic issue",
            &["bug"],
            "Search result body",
            "2026-04-20T10:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo,
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SearchKnowledgeBridge {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            query: "semantic query".to_string(),
            request_id: 9,
            selected_number: None,
        },
    );

    assert!(
        immediate_events.is_empty(),
        "semantic search must not reply on the frontend event loop"
    );
    wait_for_recorded_event("knowledge search dispatch", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.target,
                            DispatchTarget::Client(client_id) if client_id == "client-1"
                        ) && matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeSearchResults {
                                id,
                                knowledge_kind,
                                query,
                                request_id,
                                entries,
                                ..
                            } if id == &window_id
                                && *knowledge_kind == gwt::KnowledgeKind::Issue
                                && query == "semantic query"
                                && *request_id == 9
                                && entries.len() == 1
                                && entries[0].number == 42
                        )
                    })
            )
        })
    });
}

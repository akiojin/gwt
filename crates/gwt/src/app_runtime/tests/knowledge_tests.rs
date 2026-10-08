use super::*;

#[cfg(unix)]
#[test]
fn app_runtime_knowledge_search_transient_failure_stays_silent_with_retry_directive() {
    // SPEC #3170 AS-17.1 / FR-098: a typed transient semantic failure must
    // complete as KnowledgeSearchResults — cache-backed rows plus the typed
    // retry directive — and must NOT surface a correlated KnowledgeError.
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    write_fake_project_index_runtime_with_missing_issues_scope(temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let cache = Cache::new(issue_cache_root(&repo));
    cache
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Silent recovery issue",
            &["bug"],
            "Cache-backed body",
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
            query: "#42".to_string(),
            request_id: 9,
            selected_number: None,
        },
    );
    assert!(immediate_events.is_empty());

    // Wait for whichever completion the backend dispatches for request 9,
    // then require it to be the silent typed completion.
    wait_for_recorded_event("knowledge search completion", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeSearchResults { request_id, .. }
                                if *request_id == 9
                        ) || matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeError { request_id, .. }
                                if *request_id == Some(9)
                        )
                    })
            )
        })
    });
    let recorded = events.lock().expect("events lock");
    let mut saw_results = false;
    for event in recorded.iter() {
        let UserEvent::Dispatch(dispatched) = event else {
            continue;
        };
        for outbound in dispatched {
            match &outbound.event {
                BackendEvent::KnowledgeError {
                    request_id,
                    message,
                    ..
                } if *request_id == Some(9) => {
                    panic!(
                        "transient semantic failure must stay silent, got \
                         KnowledgeError: {message}"
                    );
                }
                BackendEvent::KnowledgeSearchResults {
                    request_id,
                    entries,
                    ..
                } if *request_id == 9 => {
                    saw_results = true;
                    assert_eq!(entries.len(), 1, "cache-backed rows stay usable");
                    assert_eq!(entries[0].number, 42);
                    let Some(super::super::KnowledgeWireMetadata::SemanticRetry(directive)) =
                        outbound.knowledge_wire_metadata.as_ref()
                    else {
                        panic!("typed transient failure carries the retry directive");
                    };
                    assert_eq!(directive.error_code, "INDEX_NOT_READY");
                    assert!(directive.retryable);
                    assert!(directive.retry_after_ms > 0);
                }
                _ => {}
            }
        }
    }
    assert!(saw_results, "KnowledgeSearchResults for request 9 expected");
}

#[test]
fn select_knowledge_bridge_entry_is_cache_backed_detail_only() {
    // SPEC #3170 AS-17.4 / FR-102 (T-946): an explicit selection carrying a
    // real request ID must be a background cache-backed DETAIL-ONLY path —
    // no remote refresh, no full list rebuild, and the latest related-work
    // snapshot is reused instead of a per-click full Session/Work scan.
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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let gh_marker = temp.path().join("gh-selection-marker.txt");
    let _marker = ScopedEnvVar::set("GWT_FAKE_GH_MARKER", &gh_marker);

    let repo = temp.path().join("repo");
    let issue_worktree = temp.path().join("repo-work-issue-42");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&issue_worktree).expect("create issue worktree");
    init_repo(&repo);
    let cache = Cache::new(issue_cache_root(&repo));
    cache
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Selected issue",
            &["bug"],
            "Detail body",
            "2026-04-20T10:00:00Z",
        ))
        .expect("write issue snapshot");
    cache
        .write_snapshot(&sample_issue_snapshot(
            43,
            "Other issue",
            &["bug"],
            "Other body",
            "2026-04-20T10:00:00Z",
        ))
        .expect("write issue snapshot");
    write_issue_link_store(&repo, HashMap::from([("work/issue-42".to_string(), 42)]));

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let seeded_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-42"),
        Some(&issue_worktree),
    )
    .expect("work id");
    let mut work_event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        seeded_at,
    );
    work_event.title = Some("Issue #42 first related work".to_string());
    work_event.owner = Some("Issue #42".to_string());
    work_event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-42".to_string()),
            worktree_path: Some(issue_worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, work_event)
        .expect("record work event");

    // Initial load captures the latest related-work snapshot for issue 42.
    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &window_id,
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(42),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    wait_for_knowledge_view_dispatch(&recorded_events, &window_id);
    let scans_after_full_load = runtime
        .knowledge_related_snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .full_scan_count();
    assert_eq!(scans_after_full_load, 1, "the full load publishes once");

    // Mutate the projection AFTER the snapshot: a detail-only selection must
    // reuse the snapshot rather than rescanning the Work projection.
    let second_worktree = temp.path().join("repo-work-issue-42-second");
    fs::create_dir_all(&second_worktree).expect("create second worktree");
    let second_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-42-second"),
        Some(&second_worktree),
    )
    .expect("second work id");
    let mut second_event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        second_id,
        Utc.with_ymd_and_hms(2026, 6, 20, 9, 30, 0).unwrap(),
    );
    second_event.title = Some("Issue #42 second related work".to_string());
    second_event.owner = Some("Issue #42".to_string());
    second_event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-42-second".to_string()),
            worktree_path: Some(second_worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, second_event)
        .expect("record second work event");
    write_issue_link_store(
        &repo,
        HashMap::from([
            ("work/issue-42".to_string(), 42),
            ("work/issue-42-second".to_string(), 42),
        ]),
    );
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;
    let events_before_selection = recorded_events.lock().expect("events lock").len();
    fs::write(&gh_marker, b"").ok();
    let _ = fs::remove_file(&gh_marker);

    let immediate = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SelectKnowledgeBridgeEntry {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            request_id: Some(5),
            number: 42,
        },
    );
    assert!(
        immediate.is_empty(),
        "selection must reply off the GUI event loop"
    );
    // Completion proves all selection work ran, including a wrong trailing refresh.
    drain_queued_blocking_tasks(&queued_tasks);

    let recorded = recorded_events.lock().expect("events lock");
    let mut detail_events = 0usize;
    for event in recorded[events_before_selection..].iter() {
        let UserEvent::Dispatch(dispatched) = event else {
            continue;
        };
        for outbound in dispatched {
            match &outbound.event {
                BackendEvent::KnowledgeEntries { .. } => {
                    panic!(
                        "a detail-only selection must not rebuild the full \
                         list (FR-102)"
                    );
                }
                BackendEvent::KnowledgeError { message, .. } => {
                    panic!("selection must resolve from cache, got error: {message}");
                }
                BackendEvent::KnowledgeDetail {
                    request_id, detail, ..
                } => {
                    assert_eq!(*request_id, Some(5));
                    detail_events += 1;
                    assert_eq!(detail.number, Some(42));
                    assert_eq!(detail.launch_issue_number, Some(42));
                    assert_eq!(
                        detail.related_works.len(),
                        1,
                        "selection must reuse the latest related-work \
                         snapshot instead of rescanning: {:?}",
                        detail.related_works
                    );
                    assert_eq!(
                        detail.related_works[0].title,
                        "Issue #42 first related work"
                    );
                }
                _ => {}
            }
        }
    }
    assert_eq!(detail_events, 1, "exactly one detail completion expected");
    assert_eq!(
        runtime
            .knowledge_related_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .full_scan_count(),
        scans_after_full_load,
        "explicit selection must not scan Session/Work again",
    );
    assert!(
        !gh_marker.exists(),
        "an explicit selection must never start a remote refresh (FR-102)"
    );
}

#[test]
fn select_knowledge_bridge_entry_snapshot_miss_stays_empty_without_full_scan() {
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
            42,
            "Cache-only selected issue",
            &["bug"],
            "Detail body",
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
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SelectKnowledgeBridgeEntry {
            id: window_id,
            knowledge_kind: gwt::KnowledgeKind::Issue,
            request_id: Some(17),
            number: 42,
        },
    );
    assert!(immediate.is_empty(), "selection remains off the GUI loop");
    wait_for_recorded_event("selection cache-miss detail", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| matches!(
                        &outbound.event,
                        BackendEvent::KnowledgeDetail {
                            request_id: Some(17),
                            detail,
                            ..
                        } if detail.number == Some(42) && detail.related_works.is_empty()
                    ))
            )
        })
    });
    let recorded = recorded_events.lock().expect("events lock");
    assert!(recorded.iter().all(|event| {
        !matches!(
            recorded_project_payload(event),
            UserEvent::Dispatch(dispatched)
                if dispatched.iter().any(|outbound| matches!(
                    &outbound.event,
                    BackendEvent::KnowledgeEntries { .. }
                ))
        )
    }));
    assert_eq!(
        runtime
            .knowledge_related_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .full_scan_count(),
        0,
        "snapshot miss must not fall back to a Session/Work scan",
    );
}

#[test]
fn select_pr_knowledge_bridge_entry_preserves_the_legacy_full_view_path() {
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
        "pr-1",
        repo,
        WindowPreset::Pr,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "pr-1");

    let immediate = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SelectKnowledgeBridgeEntry {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Pr,
            request_id: Some(91),
            number: 77,
        },
    );
    assert!(immediate.is_empty(), "PR selection stays off the GUI loop");
    wait_for_recorded_event("PR selection full view", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| matches!(
                        &outbound.event,
                        BackendEvent::KnowledgeEntries {
                            request_id: Some(91),
                            ..
                        } | BackendEvent::KnowledgeError {
                            request_id: Some(91),
                            ..
                        }
                    ))
            )
        })
    });

    let recorded = recorded_events.lock().expect("events lock");
    let dispatch = recorded
        .iter()
        .filter_map(|event| match event {
            UserEvent::Dispatch(dispatched) => Some(dispatched),
            _ => None,
        })
        .find(|dispatched| {
            dispatched.iter().any(|outbound| {
                matches!(
                    &outbound.event,
                    BackendEvent::KnowledgeEntries {
                        request_id: Some(91),
                        ..
                    } | BackendEvent::KnowledgeError {
                        request_id: Some(91),
                        ..
                    }
                )
            })
        })
        .expect("PR selection completion");
    assert_eq!(
        dispatch.len(),
        2,
        "legacy PR selection returns the full entries + detail view"
    );
    assert!(matches!(
        &dispatch[0].event,
        BackendEvent::KnowledgeEntries {
            knowledge_kind: gwt::KnowledgeKind::Pr,
            request_id: Some(91),
            entries,
            refresh_enabled: false,
            ..
        } if entries.is_empty()
    ));
    assert!(matches!(
        &dispatch[1].event,
        BackendEvent::KnowledgeDetail {
            knowledge_kind: gwt::KnowledgeKind::Pr,
            request_id: Some(91),
            detail,
            ..
        } if detail.sections.iter().any(|section| {
            section.body.contains("cache-backed PR list support")
        })
    ));
    assert!(dispatch
        .iter()
        .all(|outbound| !matches!(outbound.event, BackendEvent::KnowledgeError { .. })));
    assert_eq!(
        runtime
            .knowledge_related_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .full_scan_count(),
        1,
        "PR selection retains the existing full-load augmentation pass",
    );
}

#[test]
fn app_runtime_manual_knowledge_refresh_replies_through_async_dispatch() {
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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
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
        FrontendEvent::LoadKnowledgeBridge {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            request_id: Some(31),
            selected_number: Some(43),
            refresh: true,
        },
    );

    assert!(
        immediate_events.is_empty(),
        "manual refresh must not block the frontend event loop"
    );
    wait_for_recorded_event("manual knowledge refresh dispatch", &events, |events| {
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
                                knowledge_kind,
                                request_id,
                                entries,
                                selected_number,
                                ..
                            } if id == &window_id
                                && *knowledge_kind == gwt::KnowledgeKind::Issue
                                && *request_id == Some(31)
                                && *selected_number == Some(43)
                                && entries.len() == 1
                                && entries[0].number == 43
                        )
                    }) && dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeDetail {
                                id,
                                request_id,
                                detail,
                                ..
                            } if id == &window_id
                                && *request_id == Some(31)
                                && detail.number == Some(43)
                        )
                    })
            )
        })
    });
}

#[cfg(unix)]
#[test]
fn app_runtime_manual_knowledge_refresh_uses_child_bare_repo_for_workspace_home() {
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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

    let workspace_home = temp.path().join("workspace");
    let bare_repo = init_workspace_home_with_child_bare(&workspace_home);
    let expected_cwd = dunce::canonicalize(&bare_repo).expect("canonical bare repo");
    let _expected = ScopedEnvVar::set("GWT_FAKE_GH_EXPECT_CWD", &expected_cwd);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        workspace_home,
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadKnowledgeBridge {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            request_id: Some(35),
            selected_number: Some(43),
            refresh: true,
        },
    );

    assert!(
        immediate_events.is_empty(),
        "manual refresh must stay asynchronous for workspace homes"
    );
    wait_for_recorded_event(
        "workspace home knowledge refresh dispatch",
        &events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::Dispatch(dispatched)
                        if dispatched.iter().any(|outbound| {
                            matches!(
                                &outbound.event,
                                BackendEvent::KnowledgeEntries {
                                    id,
                                    knowledge_kind,
                                    request_id,
                                    entries,
                                    selected_number,
                                    ..
                                } if id == &window_id
                                    && *knowledge_kind == gwt::KnowledgeKind::Issue
                                    && *request_id == Some(35)
                                    && *selected_number == Some(43)
                                    && entries.len() == 1
                                    && entries[0].number == 43
                            )
                        })
                )
            })
        },
    );
}

#[test]
fn app_runtime_manual_knowledge_refresh_error_preserves_request_context() {
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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
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
        FrontendEvent::LoadKnowledgeBridge {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            request_id: Some(32),
            selected_number: None,
            refresh: true,
        },
    );

    assert!(
        immediate_events.is_empty(),
        "manual refresh errors must be reported asynchronously"
    );
    wait_for_recorded_event("manual knowledge refresh error", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeError {
                                id,
                                knowledge_kind,
                                request_id,
                                message,
                                ..
                            } if id == &window_id
                                && *knowledge_kind == gwt::KnowledgeKind::Issue
                                && *request_id == Some(32)
                                && message.contains("gh refresh failed")
                        )
                    })
            )
        })
    });
}

#[test]
fn app_runtime_background_knowledge_refresh_silent_paths_do_not_dispatch() {
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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let marker = temp.path().join("fake-gh-called");
    let _marker = ScopedEnvVar::set("GWT_FAKE_GH_MARKER", &marker);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    // Run each refresh to completion before asserting: the fake gh writes the
    // marker before it exits, so waiting on the marker races its still-open
    // handle (Windows os error 32) and the refresh result (Issue #4793).
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;
    let window_id = combined_window_id("tab-1", "issue-1");

    let mode_guard = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");
    runtime.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
        client_id: "client-1".to_string(),
        id: window_id.clone(),
        project_root: repo,
        kind: gwt::KnowledgeKind::Issue,
        request_id: Some(33),
        selected_number: None,
        force: false,
        sessions_dir: runtime.sessions_dir.clone(),
        issue_link_cache_dir: runtime.issue_link_cache_dir.clone(),
    });
    drain_queued_blocking_tasks(&queued_tasks);
    assert!(marker.exists(), "stale knowledge refresh must invoke gh");
    assert!(
        events.lock().expect("event log").is_empty(),
        "background refresh errors should not overwrite the current cache view"
    );

    fs::remove_file(&marker).expect("remove marker");
    drop(mode_guard);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

    runtime.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
        client_id: "client-1".to_string(),
        id: window_id,
        project_root: temp.path().join("missing-repo"),
        kind: gwt::KnowledgeKind::Issue,
        request_id: Some(34),
        selected_number: Some(43),
        force: false,
        sessions_dir: runtime.sessions_dir.clone(),
        issue_link_cache_dir: runtime.issue_link_cache_dir.clone(),
    });
    drain_queued_blocking_tasks(&queued_tasks);
    assert!(
        events.lock().expect("event log").is_empty(),
        "noop background refresh should return silently without dispatch"
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_keeps_pr_surface_disabled_until_cache_support_exists() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "pr-1",
        repo,
        WindowPreset::Pr,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "pr-1"),
            kind: gwt::KnowledgeKind::Pr,
            request_id: None,
            selected_number: None,
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "pr-1"));

    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0].event,
        BackendEvent::KnowledgeEntries {
            knowledge_kind,
            entries,
            refresh_enabled,
            empty_message,
            ..
        } if *knowledge_kind == gwt::KnowledgeKind::Pr
            && entries.is_empty()
            && !*refresh_enabled
            && empty_message.as_deref().is_some_and(|message| message.contains("cache-backed PR list support"))
    ));
    assert!(matches!(
        &events[1].event,
        BackendEvent::KnowledgeDetail { detail, .. }
            if detail.sections.iter().any(|section| section.body.contains("cache-backed PR list support"))
    ));
}

#[test]
fn app_runtime_load_profile_replies_with_config_backed_snapshot() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let config_path = temp.path().join("profile-config.toml");
    let mut settings = Settings::default();
    settings
        .profiles
        .add(Profile::new("dev"))
        .expect("add profile");
    settings.profiles.switch("dev").expect("switch active");
    settings
        .profiles
        .set_env_var("dev", "API_KEY", "override")
        .expect("set env var");
    write_profile_config(&config_path, &settings);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "profile-1",
        repo,
        WindowPreset::Profile,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "profile-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadProfile {
            id: window_id.clone(),
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::ProfileSnapshot { id, snapshot },
            ..
        }] if client_id == "client-1"
            && id == &window_id
            && snapshot.active_profile == "dev"
            && snapshot.selected_profile == "dev"
            && snapshot.profiles.iter().any(|profile|
                profile.name == "dev"
                    && profile.is_active
                    && profile.env_vars.iter().any(|entry|
                        entry.key == "API_KEY" && entry.value == "override"
                    )
            )
    ));
}

#[test]
fn app_runtime_select_and_save_profile_broadcasts_snapshot_to_profile_windows() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let config_path = temp.path().join("profile-config.toml");
    let mut settings = Settings::default();
    settings
        .profiles
        .add(Profile::new("dev"))
        .expect("add profile");
    write_profile_config(&config_path, &settings);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "profile-1",
        WindowPreset::Profile,
        WindowProcessStatus::Ready,
    ));
    persisted.windows.push(sample_window(
        "profile-2",
        WindowPreset::Profile,
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
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let current_window_id = combined_window_id("tab-1", "profile-1");
    let sibling_window_id = combined_window_id("tab-1", "profile-2");

    let select_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SelectProfile {
            id: current_window_id.clone(),
            profile_name: "dev".to_string(),
        },
    );
    assert!(matches!(
        &select_events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::ProfileSnapshot { id, snapshot },
            ..
        }] if client_id == "client-1"
            && id == &current_window_id
            && snapshot.selected_profile == "dev"
    ));

    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        temp.path(),
        vec![gwt::AgentOption {
            id: "stale-detection-sentinel".into(),
            name: "Stale detection".into(),
            available: false,
            installed_version: None,
            custom_agent: None,
        }],
    );
    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SaveProfile {
            id: current_window_id.clone(),
            current_name: "dev".to_string(),
            name: "review".to_string(),
            description: "Review profile".to_string(),
            env_vars: vec![ProfileEnvEntryView {
                key: "API_KEY".to_string(),
                value: "override".to_string(),
            }],
            disabled_env: vec!["SECRET".to_string()],
        },
    );

    assert!(
        runtime
            .launch_wizard_cache
            .agent_options()
            .iter()
            .all(|agent| { agent.id != "stale-detection-sentinel" }),
        "saving a profile must invalidate installed CLI detection"
    );
    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Project(key),
            event: BackendEvent::ProfileSnapshot { id, snapshot },
            ..
        } if key == &runtime.test_context().project_key && id == &current_window_id
            && snapshot.selected_profile == "review"
            && snapshot.active_profile == "default"
            && snapshot.profiles.iter().any(|profile|
                profile.name == "review"
                    && profile.env_vars.iter().any(|entry|
                        entry.key == "API_KEY" && entry.value == "override"
                    )
            )
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Project(key),
            event: BackendEvent::ProfileSnapshot { id, snapshot },
            ..
        } if id == &sibling_window_id
            && snapshot.selected_profile == "default"
            && snapshot.profiles.iter().any(|profile| profile.name == "review")
    )));

    let saved = Settings::load_from_path(&config_path).expect("load saved config");
    assert!(saved
        .profiles
        .profiles
        .iter()
        .any(|profile| profile.name == "review" && profile.description == "Review profile"));
}

#[test]
fn app_runtime_logs_profile_save_user_action_without_env_values() {
    let _env_lock = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _config_home = ScopedEnvVar::set("GWT_CONFIG_HOME", temp.path());
    Settings::default()
        .save(&temp.path().join("config.toml"))
        .expect("save settings");

    let mut runtime = sample_runtime(temp.path(), vec![], None);
    let events = capture_tracing_events(|| {
        let _ = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SaveProfile {
                id: "profile-window".to_string(),
                current_name: "default".to_string(),
                name: "default".to_string(),
                description: String::new(),
                env_vars: vec![ProfileEnvEntryView {
                    key: "Test".to_string(),
                    value: "must-not-leak".to_string(),
                }],
                disabled_env: vec![],
            },
        );
    });

    let action = events
        .iter()
        .find(|event| event.target == "gwt_ui_action")
        .expect("profile save user action log");
    assert_eq!(action.level, Level::INFO);
    assert_eq!(
        action.fields.get("action").map(String::as_str),
        Some("save_profile")
    );
    assert_eq!(
        action.fields.get("profile_name").map(String::as_str),
        Some("default")
    );
    assert_eq!(
        action.fields.get("env_keys").map(String::as_str),
        Some("Test")
    );
    assert_eq!(
        action.fields.get("env_var_count").map(String::as_str),
        Some("1")
    );
    assert!(
        !action
            .fields
            .values()
            .any(|value| value.contains("must-not-leak")),
        "env values must not be written to the user action log: {action:?}"
    );
}

#[test]
fn frontend_user_action_redacts_backend_test_url_secrets() {
    let custom_agent_log =
        super::super::frontend_user_action_log(&FrontendEvent::TestBackendConnection {
            base_url: "https://user:pass@example.com/v1?token=secret#frag".to_string(),
            api_key: "api-key-must-not-leak".to_string(),
        })
        .expect("custom agent backend test action log");
    assert_eq!(custom_agent_log.ui_target, "https://example.com");

    let builtin_agent_log =
        super::super::frontend_user_action_log(&FrontendEvent::TestAgentBackendConnection {
            agent: gwt_agent::BuiltinAgentId::Codex,
            base_url: "http://token@example.net:11434/openai?signed=secret".to_string(),
            api_key: "agent-key-must-not-leak".to_string(),
        })
        .expect("builtin agent backend test action log");
    assert_eq!(builtin_agent_log.ui_target, "http://example.net:11434");

    let logged_values = [
        custom_agent_log.ui_target.as_str(),
        builtin_agent_log.ui_target.as_str(),
    ];
    assert!(
        !logged_values.iter().any(|value| value.contains("user")
            || value.contains("pass")
            || value.contains("token")
            || value.contains("secret")),
        "backend test URLs must not leak credentials or query strings: {logged_values:?}"
    );
}

#[test]
fn frontend_user_action_logs_project_index_search_without_query_values() {
    let log = super::super::frontend_user_action_log(&FrontendEvent::SearchProjectIndex {
        id: "index-window".to_string(),
        query: "secret query".to_string(),
        request_id: 7,
        scopes: vec![
            gwt::IndexSearchScope::Issues,
            gwt::IndexSearchScope::FilesDocs,
        ],
        worktree_hash: Some("worktree-hash".to_string()),
        match_mode: gwt::IndexSearchMatchMode::AllTerms,
    })
    .expect("project index search user action log");

    assert_eq!(log.action, "search_project_index");
    assert_eq!(log.surface, "index");
    assert_eq!(log.window_id, "index-window");
    assert_eq!(log.mode, "files-docs,issues");
    assert_eq!(log.agent_id, "worktree-hash");
    assert_eq!(log.count, "secret query".len());

    let logged_values = [
        log.window_id.as_str(),
        log.ui_target.as_str(),
        log.profile_name.as_str(),
        log.env_keys.as_str(),
        log.agent_id.as_str(),
        log.mode.as_str(),
    ];
    assert!(
        !logged_values.iter().any(|value| value.contains("secret")),
        "project index search query must not be written to the user action log: {logged_values:?}"
    );
}

#[test]
fn frontend_user_action_logs_issue_monitor_global_profile_configure() {
    let log = super::super::frontend_user_action_log(&FrontendEvent::IssueMonitorConfigureProfile)
        .expect("issue monitor configure profile user action log");

    assert_eq!(log.action, "issue_monitor_configure_profile");
    assert_eq!(log.surface, "issue_monitor");
    assert_eq!(log.ui_target, "");
}

#[test]
fn app_runtime_load_logs_replies_with_current_log_snapshot() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "logs-1",
        repo,
        WindowPreset::Logs,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "logs-1");
    let log_path = current_log_file(&runtime.log_dir);
    // Write the canonical on-disk JSONL shape produced by
    // `tracing_subscriber::fmt::layer().json()` (see
    // `crates/gwt-core/src/logging/fmt_layer.rs`) so the reader exercises
    // the production format end-to-end (SPEC-1924 FR-035).
    fs::write(
        &log_path,
        "{\"timestamp\":\"2026-05-20T09:00:00.000000+00:00\",\
             \"level\":\"WARN\",\
             \"fields\":{\"message\":\"runtime stalled\",\"detail\":\"retrying read\"},\
             \"target\":\"pty\"}\n",
    )
    .expect("write log snapshot");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadLogs {
            id: window_id.clone(),
            scope: LogScopeSelection::default(),
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::LogEntries { id, entries },
            ..
        }] if client_id == "client-1"
            && id == &window_id
            && entries.len() == 1
            && entries[0].message == "runtime stalled"
            && entries[0].detail.as_deref() == Some("retrying read")
            && entries[0].source == "pty"
            && matches!(entries[0].severity, LogLevel::Warn)
    ));
}

#[test]
fn frontend_project_log_tab_id_routes_non_window_owners() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project_a = temp.path().join("a");
    let project_b = temp.path().join("b");
    fs::create_dir_all(&project_a).expect("project A");
    fs::create_dir_all(&project_b).expect("project B");
    let tabs = vec![
        sample_project_tab("tab-a", "A", project_a.clone(), ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-b", "B", project_b, ProjectKind::NonRepo, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-b"));
    let context_a = runtime.project_context("tab-a").unwrap();
    let context_b = runtime.project_context("tab-b").unwrap();
    runtime.project_state_mut(&context_a).unwrap().launch_wizard =
        Some(sample_launch_wizard_session("tab-a", &project_a));
    let cases = [
        (
            FrontendEvent::RebuildIndexCell {
                project_root: project_a.display().to_string(),
                scope: gwt::IndexRebuildScope::Issues,
                worktree_hash: None,
            },
            Some("tab-a"),
        ),
        (
            FrontendEvent::RefreshIndexStatus {
                project_root: project_a.display().to_string(),
            },
            Some("tab-a"),
        ),
        (
            FrontendEvent::LaunchWizardAction {
                action: gwt::LaunchWizardAction::Cancel,
                bounds: None,
            },
            Some("tab-a"),
        ),
        (
            FrontendEvent::OpenActiveWorkLaunchWizard {
                branch_name: "work/example".into(),
                linked_issue_number: None,
            },
            Some("tab-b"),
        ),
        (
            FrontendEvent::RunWorkspaceCleanup {
                branch: "work/example".into(),
                delete_remote: false,
                force_filesystem_delete: false,
                operation_id: "cleanup-op-1".to_string(),
            },
            Some("tab-b"),
        ),
        (
            FrontendEvent::RefreshIndexStatus {
                project_root: temp.path().join("unknown").display().to_string(),
            },
            None,
        ),
        (FrontendEvent::GetSystemSettings, None),
    ];
    let actual = cases
        .iter()
        .map(|(event, _)| {
            let context = if matches!(event, FrontendEvent::LaunchWizardAction { .. }) {
                &context_a
            } else {
                &context_b
            };
            runtime.frontend_project_log_tab_id(Some(context), event)
        })
        .collect::<Vec<_>>();
    let expected = cases.iter().map(|(_, owner)| *owner).collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "project operations use their explicit owner, machine operations remain global"
    );
}

/// SPEC-1924 US-17 / FR-050 / FR-054 — AppRuntime owns the lifecycle wiring
/// between project tabs and the process-wide logging router. This is one
/// integration contract because `logging::init` installs exactly one global
/// subscriber: keeping registration, snapshot resolution, and spawn-time
/// scope capture in the same test avoids a second global initialization.
#[test]
fn app_runtime_routes_restored_opened_and_queued_logs_by_project_scope() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let machine_log_dir = temp.path().join("machine-logs");
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    fs::create_dir_all(&project_a).expect("create project A");
    fs::create_dir_all(&project_b).expect("create project B");
    fs::create_dir_all(&machine_log_dir).expect("create machine log dir");
    fs::write(
        current_log_file(&machine_log_dir),
        "{\"timestamp\":\"2026-08-29T00:00:00Z\",\"level\":\"INFO\",\"fields\":{\"message\":\"snapshot-global\"},\"target\":\"gwt::logging_contract\"}\n",
    )
    .expect("write global log snapshot before the writer opens it");

    let mut logging = init_logging(LoggingConfig {
        log_dir: machine_log_dir,
        default_level: LogLevel::Info,
        config_file_level: Some(LogLevel::Info),
        retention_days: 0,
    })
    .expect("initialize isolated logging router");
    logging
        .set_level(LogLevel::Info)
        .expect("make contract markers observable regardless of RUST_LOG");
    let mut live_logs = logging.take_ui_rx().expect("live log receiver");
    let router = logging.router();

    let restored_tab = sample_project_tab_with_window_at(
        "tab-a",
        "logs-a",
        project_a.clone(),
        WindowPreset::Logs,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![restored_tab], Some("tab-a"));
    // The setter registers tabs restored before the router becomes available.
    runtime.set_project_log_router(router);

    let scope_a = runtime
        .project_log_scope_for_tab("tab-a")
        .expect("restored project is registered")
        .clone();
    assert_eq!(
        scope_a.log_dir(),
        gwt_core::paths::gwt_project_logs_dir_for_project_path(&project_a),
        "restored project uses its canonical project log store"
    );

    runtime.open_project_path_events(project_b.clone());
    let prepared = take_project_navigation_completion(&recorded_events);
    runtime.handle_project_navigation_prepared(prepared);
    let tab_b_id = runtime
        .active_tab_id
        .clone()
        .expect("opened project becomes active");
    let scope_b = runtime
        .project_log_scope_for_tab(&tab_b_id)
        .expect("opened project is registered")
        .clone();
    assert_eq!(
        scope_b.log_dir(),
        gwt_core::paths::gwt_project_logs_dir_for_project_path(&project_b),
        "opened project uses its canonical project log store"
    );

    let context_b = runtime.project_context(&tab_b_id).unwrap();
    runtime.create_window_events(&context_b, WindowPreset::Logs, canvas_bounds());
    let logs_b_raw_id = runtime
        .tab(&tab_b_id)
        .expect("opened project tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Logs)
        .expect("project B Logs surface")
        .id
        .clone();
    let logs_a_id = combined_window_id("tab-a", "logs-a");
    let logs_b_id = combined_window_id(&tab_b_id, &logs_b_raw_id);

    for (log_dir, marker) in [
        (scope_a.log_dir(), "snapshot-project-a"),
        (scope_b.log_dir(), "snapshot-project-b"),
    ] {
        fs::create_dir_all(log_dir).expect("create project log snapshot dir");
        fs::write(
            current_log_file(log_dir),
            format!(
                "{{\"timestamp\":\"2026-08-29T00:00:00Z\",\"level\":\"INFO\",\"fields\":{{\"message\":\"{marker}\"}},\"target\":\"gwt::logging_contract\"}}\n"
            ),
        )
        .expect("write project log snapshot");
    }

    let loaded_entries = |window_id: &str| {
        runtime
            .load_logs_events("client-1", window_id)
            .into_iter()
            .find_map(|event| match event.event {
                BackendEvent::LogEntries { entries, .. } => Some(entries),
                _ => None,
            })
            .expect("Logs surface receives a snapshot")
    };
    let entries_a = loaded_entries(&logs_a_id);
    assert_eq!(entries_a[0].message, "snapshot-project-a");
    assert_eq!(
        entries_a[0].project_scope.as_deref(),
        Some(scope_a.as_str()),
        "legacy project-file records inherit the selected project scope"
    );
    let entries_b = loaded_entries(&logs_b_id);
    assert_eq!(entries_b[0].message, "snapshot-project-b");
    assert_eq!(
        entries_b[0].project_scope.as_deref(),
        Some(scope_b.as_str()),
        "legacy project-file records inherit the selected project scope"
    );

    let context_a = runtime.project_context("tab-a").unwrap();
    let global_entries = runtime
        .handle_frontend_event_for_project(
            &context_a,
            "client-1".to_string(),
            FrontendEvent::LoadLogs {
                id: logs_a_id.clone(),
                scope: LogScopeSelection::Global,
            },
        )
        .into_iter()
        .find_map(|event| match event.event {
            BackendEvent::LogEntries { entries, .. } => Some(entries),
            _ => None,
        })
        .expect("Global Logs facet receives the machine snapshot");
    assert!(global_entries
        .iter()
        .any(|entry| { entry.message == "snapshot-global" && entry.project_scope.is_none() }));
    assert!(global_entries.iter().all(|entry| {
        entry.message != "snapshot-project-a" && entry.message != "snapshot-project-b"
    }));

    runtime.select_project_tab_events("tab-a");
    let ui_trace_path = runtime
        .handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SaveUiTrace {
                trace: serde_json::from_value::<UiTracePayload>(serde_json::json!({
                    "session_id": "project-a-trace",
                    "entries": [{ "kind": "trace_start", "ts": 1 }]
                }))
                .expect("typed project UI trace payload"),
            },
        )
        .into_iter()
        .find_map(|event| match event.event {
            BackendEvent::UiTraceSaved { path, .. } => Some(PathBuf::from(path)),
            _ => None,
        })
        .expect("project UI trace is saved");
    assert_eq!(
        ui_trace_path.parent(),
        Some(scope_a.log_dir()),
        "UI traces follow the active project's canonical log store"
    );

    // The connection owns the artifact even while the legacy test selection is A.
    let project_b_trace = runtime.handle_frontend_event_for_project(
        &context_b,
        "client-b".to_string(),
        FrontendEvent::SaveUiTrace {
            trace: serde_json::from_value::<UiTracePayload>(serde_json::json!({
                "session_id": "project-b-trace",
                "entries": [{ "kind": "trace_start", "ts": 1 }]
            }))
            .expect("typed B trace"),
        },
    );
    assert!(matches!(
        &project_b_trace[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::UiTraceSaved { path, entries }, ..
        }] if client_id == "client-b" && *entries == 1
            && Path::new(path).exists() && Path::new(path).parent() == Some(scope_b.log_dir())
    ));

    runtime.set_active_tab(tab_b_id.clone());
    assert_eq!(
        runtime
            .frontend_project_log_scope(
                Some(&context_a),
                &FrontendEvent::FocusWindow {
                    id: logs_a_id.clone(),
                    bounds: Some(canvas_bounds()),
                }
            )
            .as_ref(),
        Some(&scope_a),
        "an event from an inactive project keeps the window owner's scope"
    );
    assert!(runtime
        .frontend_project_log_scope(Some(&context_a), &FrontendEvent::GetSystemSettings)
        .is_none());
    assert!(runtime
        .frontend_project_log_scope(
            Some(&context_a),
            &FrontendEvent::LoadLogs {
                id: logs_a_id.clone(),
                scope: LogScopeSelection::Global,
            }
        )
        .is_none());

    // Capture A at enqueue time, then switch the active project before the
    // queued work runs. Running it inside B's foreground scope makes the
    // regression unambiguous: the spawn-time A scope must win for the task.
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    {
        let _scope = scope_a.enter();
        runtime.blocking_tasks.spawn(|| {
            tracing::info!(
                target: "gwt::logging_contract",
                "queued-project-a-after-switch"
            );
        });
    }
    runtime.blocking_tasks.spawn(|| {
        tracing::info!(target: "gwt::logging_contract", "queued-global-after-switch");
    });
    runtime.select_project_tab_events(&tab_b_id);
    let queued_a = tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(0);
    {
        let _scope = scope_b.enter();
        tracing::info!(target: "gwt::logging_contract", "foreground-project-b");
        queued_a();
        let queued_global = tasks.lock().expect("queue").remove(0);
        queued_global();
    }

    let worker_runtime = tokio::runtime::Runtime::new().expect("worker runtime");
    for (spawner, marker) in [
        (
            BlockingTaskSpawner::thread(),
            "thread-project-a-after-switch",
        ),
        (
            BlockingTaskSpawner::tokio(worker_runtime.handle().clone()),
            "tokio-project-a-after-switch",
        ),
    ] {
        let (done, finished) = std::sync::mpsc::channel();
        {
            let _scope = scope_a.enter();
            spawner.spawn(move || {
                tracing::info!(target: "gwt::logging_contract", "{marker}");
                done.send(()).expect("worker completed");
            });
        }
        finished
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("worker log received");
    }

    let error_tab = sample_project_tab_with_window_at(
        "tab-error",
        "agent-error",
        project_a.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut error_runtime, error_events) =
        sample_runtime_with_events(temp.path(), vec![error_tab], Some("tab-error"));
    error_runtime.set_project_log_router(logging.router());
    let error_id = combined_window_id("tab-error", "agent-error");
    {
        let _other_project = scope_b.enter();
        error_runtime.observe_provider_api_error(&error_id, Some(CLAUDE_API_ERROR_SCREEN));
    }

    error_runtime.close_project_tab_events("tab-error");
    error_runtime.open_project_path_events(project_a.clone());
    let reopened = take_project_navigation_completion(&error_events);
    error_runtime.handle_project_navigation_prepared(reopened);
    assert_eq!(
        error_runtime
            .active_tab_id
            .as_deref()
            .and_then(|id| error_runtime.project_log_scope_for_tab(id)),
        Some(&scope_a),
        "reopened projects recover the same canonical logging scope"
    );

    let migrated = temp.path().join("project-a-migrated");
    fs::create_dir_all(&migrated).expect("create migrated project");
    runtime.handle_migration_done(&context_a, &migrated);
    let migrated_scope = runtime
        .project_log_scope_for_tab("tab-a")
        .expect("migrated scope");
    assert_eq!(
        migrated_scope.log_dir(),
        gwt_core::paths::gwt_project_logs_dir_for_project_path(&migrated)
    );
    assert_ne!(migrated_scope.as_str(), scope_a.as_str());

    let live = std::iter::from_fn(|| live_logs.try_recv().ok()).collect::<Vec<_>>();
    assert!(
        live.iter().any(|event| {
            event
                .message
                .contains("a provider API error ended this pane")
                && event.project_scope.as_deref() == Some(scope_a.as_str())
        }),
        "runtime diagnostics use the affected pane's scope even under another project"
    );
    assert!(
        live.iter().any(|event| {
            event.message == "queued-project-a-after-switch"
                && event.project_scope.as_deref() == Some(scope_a.as_str())
        }),
        "queued A scope missing: {:?}",
        live.iter()
            .filter(|entry| entry.source == "gwt::logging_contract")
            .map(|entry| (&entry.message, &entry.project_scope))
            .collect::<Vec<_>>()
    );
    assert!(
        live.iter().any(|event| {
            event.message == "queued-global-after-switch" && event.project_scope.is_none()
        }),
        "a task queued without a project must not inherit the executor's active scope"
    );
    for marker in [
        "thread-project-a-after-switch",
        "tokio-project-a-after-switch",
    ] {
        assert!(
            live.iter().any(|event| event.message == marker
                && event.project_scope.as_deref() == Some(scope_a.as_str())),
            "{marker} preserves spawn scope"
        );
    }
    assert!(live.iter().any(|event| {
        event.message == "foreground-project-b"
            && event.project_scope.as_deref() == Some(scope_b.as_str())
    }));
}

/// SPEC-1924 US-14 / FR-036 / SC-010 — when canonical log file contains
/// malformed lines, the Logs window receives the surviving entries plus
/// exactly one Warning notice via `LogEntryAppended`.
#[test]
fn app_runtime_load_logs_emits_warning_for_skipped_lines() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "logs-1",
        repo,
        WindowPreset::Logs,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "logs-1");
    let log_path = current_log_file(&runtime.log_dir);
    let good = "{\"timestamp\":\"2026-05-20T09:00:00.000000+00:00\",\
            \"level\":\"INFO\",\"fields\":{\"message\":\"ok\"},\"target\":\"gwt\"}";
    let malformed = "{\"foo\":\"bar\"}";
    fs::write(&log_path, format!("{good}\n{malformed}\n{good}\n")).expect("write log snapshot");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadLogs {
            id: window_id.clone(),
            scope: LogScopeSelection::default(),
        },
    );

    assert_eq!(
        events.len(),
        2,
        "expected LogEntries + LogEntryAppended for skipped notice, got {:?}",
        events
    );

    let entries_match = matches!(
        &events[0],
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::LogEntries { id, entries },
            ..
        } if client_id == "client-1"
            && id == &window_id
            && entries.len() == 2
            && entries.iter().all(|e| e.message == "ok")
    );
    assert!(
        entries_match,
        "first event must be LogEntries: {:?}",
        events[0]
    );

    let warning_match = matches!(
        &events[1],
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::LogEntryAppended { entry },
            ..
        } if client_id == "client-1"
            && entry.severity == LogLevel::Warn
            && entry.source == "gwt_core::logging::reader"
            && entry.message.contains("Skipped 1 malformed line")
    );
    assert!(
        warning_match,
        "second event must be a Warn LogEntryAppended notice: {:?}",
        events[1]
    );
}

#[test]
fn app_runtime_save_ui_trace_replies_with_artifact_path() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), vec![], None);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SaveUiTrace {
            trace: serde_json::from_value::<UiTracePayload>(serde_json::json!({
                "session_id": "trace-1",
                "entries": [
                    { "kind": "trace_start", "ts": 1 }
                ]
            }))
            .expect("typed ui trace payload"),
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::UiTraceSaved { path, entries },
            ..
        }] if client_id == "client-1" && *entries == 1 && Path::new(path).exists()
    ));
}

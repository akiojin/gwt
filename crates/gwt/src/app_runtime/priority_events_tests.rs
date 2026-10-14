use super::{AppEventQueue, ProjectContext, UserEvent};
use gwt::{FrontendEvent, KnowledgeKind, WindowGeometry};

fn frontend(event: FrontendEvent) -> UserEvent {
    UserEvent::Frontend {
        client_id: "client".to_string(),
        client_scope: None,
        event,
        received_at: std::time::Instant::now(),
    }
}

fn geometry() -> WindowGeometry {
    WindowGeometry {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    }
}

fn grid(client_id: &str, id: &str, cols: u16, received_at: std::time::Instant) -> UserEvent {
    UserEvent::Frontend {
        client_id: client_id.into(),
        client_scope: Some(super::ClientScope::Project(
            gwt_core::repo_hash::ProjectKey::parse("0123456789abcdef").unwrap(),
        )),
        event: FrontendEvent::UpdateTerminalGrid {
            id: id.into(),
            cols,
            rows: 24,
        },
        received_at,
    }
}

fn scoped(event: UserEvent, generation: u64) -> UserEvent {
    UserEvent::ProjectCompletion {
        context: ProjectContext {
            tab_id: "project-tab".into(),
            project_key: gwt_core::repo_hash::ProjectKey::parse("0123456789abcdef").unwrap(),
            generation,
            project_root: "/project".into(),
        },
        event: Box::new(event),
    }
}

#[test]
fn terminal_grid_queue_coalesces_at_latest_arrival_position() {
    let received_at = std::time::Instant::now();
    let mut queue = AppEventQueue::default();
    queue
        .push_back(grid("client", "window", 80, received_at))
        .unwrap();
    queue
        .push_back(frontend(FrontendEvent::UpdateWindowGeometry {
            id: "window".into(),
            geometry: geometry(),
            cols: 100,
            rows: 24,
            base_geometry_revision: None,
        }))
        .unwrap();
    queue
        .push_back(frontend(FrontendEvent::TerminalInput {
            id: "window".into(),
            data: "input".into(),
        }))
        .unwrap();
    // Equal timestamps still keep the later arrival, after the geometry commit.
    queue
        .push_back(grid("client", "window", 120, received_at))
        .unwrap();
    assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
        event: FrontendEvent::TerminalInput { data, .. }, ..
    }) if data == "input"));
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::UpdateWindowGeometry { cols: 100, .. },
            ..
        })
    ));
    assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
        event: FrontendEvent::UpdateTerminalGrid { cols: 120, .. },
        received_at: delivered_at, ..
    }) if delivered_at == received_at));
    assert!(queue.is_empty());
}

#[test]
fn terminal_grid_queue_coalesces_clients_and_keeps_window_and_project_scope_separate() {
    let received_at = std::time::Instant::now();
    let mut queue = AppEventQueue::default();
    let mut other_project = grid("other-client", "window", 82, received_at);
    if let UserEvent::Frontend { client_scope, .. } = &mut other_project {
        *client_scope = Some(super::ClientScope::Project(
            gwt_core::repo_hash::ProjectKey::parse("fedcba9876543210").unwrap(),
        ));
    }
    queue
        .push_back(scoped(
            scoped(grid("client", "window", 80, received_at), 9),
            7,
        ))
        .unwrap();
    let distinct = [
        grid("client", "window", 81, received_at),
        scoped(scoped(other_project, 9), 7),
        scoped(
            scoped(grid("client", "other-window", 83, received_at), 9),
            7,
        ),
        scoped(scoped(grid("client", "window", 84, received_at), 9), 8),
        scoped(scoped(grid("client", "window", 85, received_at), 10), 7),
    ];
    for event in &distinct {
        queue.push_back(event.clone()).unwrap();
    }
    let latest = scoped(
        scoped(grid("other-client", "window", 120, received_at), 9),
        7,
    );
    queue.push_back(latest.clone()).unwrap();
    for expected in distinct.into_iter().chain([latest]) {
        let delivered = queue.pop_front().expect("independent grid update");
        assert_eq!(format!("{delivered:?}"), format!("{expected:?}"));
    }
    assert!(queue.is_empty());
}

#[test]
fn terminal_grid_queue_replay_cannot_replace_newer_pending_update() {
    let received_at = std::time::Instant::now();
    let old = received_at - std::time::Duration::from_secs(1);
    let arrange = frontend(FrontendEvent::ArrangeWindows {
        mode: gwt::protocol::ArrangeMode::Tile,
        bounds: geometry(),
    });
    let mut queue = AppEventQueue::default();
    queue
        .push_back(grid("client", "window", 120, received_at))
        .unwrap();
    queue.prepend(vec![
        grid("client", "window", 80, old),
        arrange.clone(),
        grid("client", "window", 100, received_at),
    ]);
    // A late delivery also cannot replace a newer receive timestamp.
    queue.push_back(grid("client", "window", 90, old)).unwrap();
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::ArrangeWindows { .. },
            ..
        })
    ));
    assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
        event: FrontendEvent::UpdateTerminalGrid { cols: 120, .. },
        received_at: delivered_at, ..
    }) if delivered_at == received_at));
    assert!(queue.is_empty());

    queue.prepend(vec![
        grid("client", "window", 80, old),
        arrange,
        grid("client", "window", 100, received_at),
    ]);
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::ArrangeWindows { .. },
            ..
        })
    ));
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::UpdateTerminalGrid { cols: 100, .. },
            ..
        })
    ));
    assert!(queue.is_empty());
}

#[test]
fn terminal_grid_queue_input_and_layout_overtake_background_without_starvation() {
    let mut queue = AppEventQueue::default();
    queue
        .push_back(UserEvent::LaunchProgress {
            window_id: "launch".into(),
            message: "background".into(),
        })
        .unwrap();
    queue
        .push_back(grid("client", "window", 80, std::time::Instant::now()))
        .unwrap();
    for index in 0..10 {
        queue
            .push_back(frontend(FrontendEvent::TerminalInput {
                id: "window".into(),
                data: index.to_string(),
            }))
            .unwrap();
    }
    for index in 0..8 {
        assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
            event: FrontendEvent::TerminalInput { data, .. }, ..
        }) if data == index.to_string()));
    }
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::LaunchProgress { .. })
    ));
    assert!(
        matches!(
            queue.pop_front(),
            Some(UserEvent::Frontend {
                event: FrontendEvent::UpdateTerminalGrid { .. },
                ..
            })
        ),
        "background service must not reset the control burst and starve layout"
    );
    for index in 8..10 {
        assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
            event: FrontendEvent::TerminalInput { data, .. }, ..
        }) if data == index.to_string()));
    }
    assert!(queue.is_empty());
}

#[test]
fn terminal_grid_queue_layout_precedes_background_with_bounded_burst() {
    let mut queue = AppEventQueue::default();
    queue
        .push_back(UserEvent::LaunchProgress {
            window_id: "launch".into(),
            message: "background".into(),
        })
        .unwrap();
    queue
        .push_back(frontend(FrontendEvent::ArrangeWindows {
            mode: gwt::protocol::ArrangeMode::Tile,
            bounds: geometry(),
        }))
        .unwrap();
    queue
        .push_back(frontend(FrontendEvent::UpdateWindowGeometry {
            id: "window".into(),
            geometry: geometry(),
            cols: 80,
            rows: 24,
            base_geometry_revision: None,
        }))
        .unwrap();
    for index in 0..7 {
        queue
            .push_back(grid(
                "client",
                &index.to_string(),
                80,
                std::time::Instant::now(),
            ))
            .unwrap();
    }
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::ArrangeWindows { .. },
            ..
        })
    ));
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::UpdateWindowGeometry { .. },
            ..
        })
    ));
    for index in 0..6 {
        assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
            event: FrontendEvent::UpdateTerminalGrid { id, .. }, ..
        }) if id == index.to_string()));
    }
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::LaunchProgress { .. })
    ));
    assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
        event: FrontendEvent::UpdateTerminalGrid { id, .. }, ..
    }) if id == "6"));
    assert!(queue.is_empty());
}

#[test]
fn launch_priority_queue_controls_overtake_background_and_keep_fifo() {
    let mut queue = AppEventQueue::default();
    queue
        .push_back(frontend(FrontendEvent::LoadKnowledgeBridge {
            id: "knowledge".to_string(),
            knowledge_kind: KnowledgeKind::Issue,
            request_id: None,
            selected_number: None,
            refresh: false,
        }))
        .unwrap();
    queue
        .push_back(UserEvent::LaunchProgress {
            window_id: "launch".to_string(),
            message: "background".to_string(),
        })
        .unwrap();
    let controls = [
        FrontendEvent::LoadPmConversation {
            id: "pm".to_string(),
        },
        FrontendEvent::FocusWindow {
            id: "focus".to_string(),
            bounds: None,
        },
        FrontendEvent::ActivateWindowTab {
            id: "activate".to_string(),
        },
        FrontendEvent::DockWindowTab {
            id: "dock".to_string(),
            target_id: "target".to_string(),
        },
    ];
    for event in &controls {
        queue.push_back(frontend(event.clone())).unwrap();
    }
    for expected in controls {
        let Some(UserEvent::Frontend { event, .. }) = queue.pop_front() else {
            panic!("queued control must overtake launch background");
        };
        assert_eq!(format!("{event:?}"), format!("{expected:?}"));
    }
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::LoadKnowledgeBridge { .. },
            ..
        })
    ));
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::LaunchProgress { .. })
    ));
    assert!(queue.is_empty());
}

#[test]
fn launch_priority_queue_serves_background_after_eight_controls() {
    let mut queue = AppEventQueue::default();
    queue
        .push_back(UserEvent::LaunchProgress {
            window_id: "launch".to_string(),
            message: "background".to_string(),
        })
        .unwrap();
    for index in 0..12 {
        queue
            .push_back(frontend(FrontendEvent::ActivateWindowTab {
                id: index.to_string(),
            }))
            .unwrap();
    }
    for expected in 0..8 {
        assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
            event: FrontendEvent::ActivateWindowTab { id }, ..
        }) if id == expected.to_string()));
    }
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::LaunchProgress { .. })
    ));
    assert!(matches!(queue.pop_front(), Some(UserEvent::Frontend {
        event: FrontendEvent::ActivateWindowTab { id }, ..
    }) if id == "8"));
}

#[test]
fn launch_priority_queue_preserves_project_context_when_prioritizing() {
    let context = ProjectContext {
        tab_id: "project-tab".to_string(),
        project_key: gwt_core::repo_hash::ProjectKey::parse("0123456789abcdef").unwrap(),
        generation: 7,
        project_root: std::path::PathBuf::from("/project"),
    };
    let mut queue = AppEventQueue::default();
    queue
        .push_back(UserEvent::LaunchProgress {
            window_id: "launch".to_string(),
            message: "background".to_string(),
        })
        .unwrap();
    queue
        .push_back(UserEvent::ProjectCompletion {
            context: context.clone(),
            event: Box::new(frontend(FrontendEvent::LoadPmConversation {
                id: "pm".to_string(),
            })),
        })
        .unwrap();
    let Some(UserEvent::ProjectCompletion {
        context: delivered_context,
        event,
    }) = queue.pop_front()
    else {
        panic!("project-scoped control must overtake launch background");
    };
    assert_eq!(delivered_context, context);
    assert!(matches!(
        *event,
        UserEvent::Frontend {
            event: FrontendEvent::LoadPmConversation { .. },
            ..
        }
    ));
}

#[test]
fn launch_priority_queue_pane_observation_overtakes_layout_backlog() {
    let project = tempfile::tempdir().unwrap();
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
    let grant = gwt::agent_capability::AgentCapabilityGrant::new(
        "test-capability".to_string(),
        gwt::agent_capability::AgentSessionPrincipal::for_test(project.path(), "session").unwrap(),
    );
    let mut queue = AppEventQueue::default();
    queue
        .push_back(frontend(FrontendEvent::ArrangeWindows {
            mode: gwt::protocol::ArrangeMode::Tile,
            bounds: geometry(),
        }))
        .unwrap();
    queue
        .push_back(frontend(FrontendEvent::UpdateWindowGeometry {
            id: "window".to_string(),
            geometry: geometry(),
            cols: 80,
            rows: 24,
            base_geometry_revision: None,
        }))
        .unwrap();
    queue
        .push_back(UserEvent::AgentFrontend {
            client_id: "agent".to_string(),
            grant,
            request: gwt::agent_capability::AgentFrontendRequest::ListWindows,
        })
        .unwrap();
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::AgentFrontend {
            request: gwt::agent_capability::AgentFrontendRequest::ListWindows,
            ..
        })
    ));
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::ArrangeWindows { .. },
            ..
        })
    ));
    assert!(matches!(
        queue.pop_front(),
        Some(UserEvent::Frontend {
            event: FrontendEvent::UpdateWindowGeometry { .. },
            ..
        })
    ));
}

#[test]
fn launch_priority_queue_close_rejects_and_moves_payloads_outside_lock() {
    let payload: std::sync::Arc<str> = "owned worker payload".into();
    let queue = std::sync::Mutex::new(AppEventQueue::default());
    let mut pending = queue.lock().unwrap();
    pending.wake_pending = true;
    pending
        .push_back(UserEvent::PreparedActiveWorkDispatch {
            context: ProjectContext {
                tab_id: "project-tab".into(),
                project_key: gwt_core::repo_hash::ProjectKey::parse("0123456789abcdef").unwrap(),
                generation: 7,
                project_root: "/project".into(),
            },
            tab_id: "project-tab".into(),
            target: super::DispatchTarget::Project(
                gwt_core::repo_hash::ProjectKey::parse("0123456789abcdef").unwrap(),
            ),
            payload: payload.clone(),
        })
        .unwrap();
    let discarded = pending.close();
    assert!(pending.closed);
    assert!(!pending.wake_pending);
    assert!(pending.is_empty());
    assert_eq!(
        std::sync::Arc::strong_count(&payload),
        2,
        "closing only transfers ownership while locked"
    );
    let error = pending
        .push_back(UserEvent::LaunchProgress {
            window_id: "rejected".into(),
            message: "exact input".into(),
        })
        .unwrap_err();
    assert!(
        matches!(error.0, UserEvent::LaunchProgress { window_id, message } if window_id == "rejected" && message == "exact input")
    );
    drop(pending);
    assert!(queue.try_lock().is_ok());
    drop(discarded);
    assert_eq!(
        std::sync::Arc::strong_count(&payload),
        1,
        "pending payload is released outside the lock"
    );
}

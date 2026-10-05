use super::{AppEventQueue, ProjectContext, UserEvent};
use gwt::{FrontendEvent, KnowledgeKind, WindowGeometry};

fn frontend(event: FrontendEvent) -> UserEvent {
    UserEvent::Frontend {
        client_id: "client".to_string(),
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

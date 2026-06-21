use gwt::{AgentKanbanLane, FrontendEvent, WindowPreset, WindowSurface};
use serde_json::json;

// SPEC-2008 FR-096/FR-097: the canvas window model dropped manual
// maximize/minimize/restore in favor of frontend-driven Camera focus. The
// legacy `maximize_window` / `minimize_window` / `restore_window` commands are
// no longer part of the [`FrontendEvent`] contract, so payloads carrying those
// `kind` values must be rejected as unknown variants.
#[test]
fn frontend_event_rejects_removed_maximize_minimize_restore_commands() {
    let maximize = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "maximize_window",
        "id": "project-1::claude-1",
        "bounds": {
            "x": 12.0,
            "y": 24.0,
            "width": 1280.0,
            "height": 720.0
        }
    }));
    assert!(
        maximize.is_err(),
        "maximize_window must no longer deserialize into a FrontendEvent: {maximize:?}"
    );

    let minimize = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "minimize_window",
        "id": "project-1::claude-1"
    }));
    assert!(
        minimize.is_err(),
        "minimize_window must no longer deserialize into a FrontendEvent: {minimize:?}"
    );

    let restore = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "restore_window",
        "id": "project-1::claude-1"
    }));
    assert!(
        restore.is_err(),
        "restore_window must no longer deserialize into a FrontendEvent: {restore:?}"
    );
}

#[test]
fn frontend_event_deserializes_window_state_commands() {
    let list = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "list_windows"
    }))
    .expect("list_windows should deserialize");
    match list {
        FrontendEvent::ListWindows => {}
        other => panic!("unexpected event: {other:?}"),
    }

    let dock = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "dock_window_tab",
        "id": "project-1::board-1",
        "target_id": "project-1::agent-1"
    }))
    .expect("dock_window_tab should deserialize");
    match dock {
        FrontendEvent::DockWindowTab { id, target_id } => {
            assert_eq!(id, "project-1::board-1");
            assert_eq!(target_id, "project-1::agent-1");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let activate = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "activate_window_tab",
        "id": "project-1::board-1"
    }))
    .expect("activate_window_tab should deserialize");
    match activate {
        FrontendEvent::ActivateWindowTab { id } => {
            assert_eq!(id, "project-1::board-1");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let detach = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "detach_window_tab",
        "id": "project-1::board-1",
        "geometry": {
            "x": 64.0,
            "y": 80.0,
            "width": 720.0,
            "height": 420.0
        }
    }))
    .expect("detach_window_tab should deserialize");
    match detach {
        FrontendEvent::DetachWindowTab { id, geometry } => {
            assert_eq!(id, "project-1::board-1");
            assert_eq!(geometry.x, 64.0);
            assert_eq!(geometry.y, 80.0);
            assert_eq!(geometry.width, 720.0);
            assert_eq!(geometry.height, 420.0);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let knowledge = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "load_knowledge_bridge",
        "id": "project-1::issue-1",
        "knowledge_kind": "issue",
        "request_id": 7,
        "selected_number": 2017,
        "refresh": true
    }))
    .expect("load_knowledge_bridge should deserialize");
    match knowledge {
        FrontendEvent::LoadKnowledgeBridge {
            id,
            knowledge_kind,
            request_id,
            selected_number,
            refresh,
        } => {
            assert_eq!(id, "project-1::issue-1");
            assert_eq!(knowledge_kind, gwt::KnowledgeKind::Issue);
            assert_eq!(request_id, Some(7));
            assert_eq!(selected_number, Some(2017));
            assert!(refresh);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let launch = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "open_issue_launch_wizard",
        "id": "project-1::issue-1",
        "issue_number": 2017
    }))
    .expect("open_issue_launch_wizard should deserialize");
    match launch {
        FrontendEvent::OpenIssueLaunchWizard { id, issue_number } => {
            assert_eq!(id, "project-1::issue-1");
            assert_eq!(issue_number, 2017);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn frontend_event_deserializes_agent_kanban_commands() {
    let place = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "place_agent_window_in_kanban",
        "id": "project-1::agent-1",
        "board_id": "project-1::agent-kanban-1",
        "lane_id": "active",
        "order": 2
    }))
    .expect("place_agent_window_in_kanban should deserialize");
    match place {
        FrontendEvent::PlaceAgentWindowInKanban {
            id,
            board_id,
            lane_id,
            order,
        } => {
            assert_eq!(id, "project-1::agent-1");
            assert_eq!(board_id, "project-1::agent-kanban-1");
            assert_eq!(lane_id, AgentKanbanLane::Active);
            assert_eq!(order, Some(2));
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let move_card = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "move_agent_kanban_card",
        "id": "project-1::agent-1",
        "board_id": "project-1::agent-kanban-1",
        "lane_id": "blocked",
        "order": 0
    }))
    .expect("move_agent_kanban_card should deserialize");
    match move_card {
        FrontendEvent::MoveAgentKanbanCard {
            id,
            board_id,
            lane_id,
            order,
        } => {
            assert_eq!(id, "project-1::agent-1");
            assert_eq!(board_id, "project-1::agent-kanban-1");
            assert_eq!(lane_id, AgentKanbanLane::Blocked);
            assert_eq!(order, 0);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let undock = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "undock_agent_window",
        "id": "project-1::agent-1",
        "geometry": {
            "x": 120.0,
            "y": 96.0,
            "width": 720.0,
            "height": 420.0
        }
    }))
    .expect("undock_agent_window should deserialize");
    match undock {
        FrontendEvent::UndockAgentWindow { id, geometry } => {
            assert_eq!(id, "project-1::agent-1");
            let geometry = geometry.expect("geometry should be present");
            assert_eq!(geometry.x, 120.0);
            assert_eq!(geometry.y, 96.0);
            assert_eq!(geometry.width, 720.0);
            assert_eq!(geometry.height, 420.0);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let collapsed = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "set_agent_kanban_card_collapsed",
        "id": "project-1::agent-1",
        "collapsed": true
    }))
    .expect("set_agent_kanban_card_collapsed should deserialize");
    match collapsed {
        FrontendEvent::SetAgentKanbanCardCollapsed { id, collapsed } => {
            assert_eq!(id, "project-1::agent-1");
            assert!(collapsed);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let grid = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "update_terminal_grid",
        "id": "project-1::agent-1",
        "cols": 101,
        "rows": 37
    }))
    .expect("update_terminal_grid should deserialize");
    match grid {
        FrontendEvent::UpdateTerminalGrid { id, cols, rows } => {
            assert_eq!(id, "project-1::agent-1");
            assert_eq!(cols, 101);
            assert_eq!(rows, 37);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn agent_kanban_preset_uses_singleton_surface_contract() {
    let preset_json = serde_json::to_value(WindowPreset::AgentKanban).expect("serialize preset");
    assert_eq!(preset_json, json!("agent_kanban"));
    assert_eq!(WindowPreset::AgentKanban.title(), "Agent Kanban");
    assert_eq!(WindowPreset::AgentKanban.id_prefix(), "agent-kanban");
    assert_eq!(
        WindowPreset::AgentKanban.surface(),
        WindowSurface::AgentKanban
    );
    assert!(!WindowPreset::AgentKanban.requires_process());
}

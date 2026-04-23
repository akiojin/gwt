use super::{AppRuntime, OutboundEvent};

pub(super) fn load_board_events(
    runtime: &AppRuntime,
    client_id: &str,
    id: &str,
) -> Vec<OutboundEvent> {
    let Some(address) = runtime.window_lookup.get(id) else {
        return vec![OutboundEvent::reply(
            client_id,
            gwt::BackendEvent::BoardError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            },
        )];
    };
    let Some(tab) = runtime.tab(&address.tab_id) else {
        return vec![OutboundEvent::reply(
            client_id,
            gwt::BackendEvent::BoardError {
                id: id.to_string(),
                message: "Project tab not found".to_string(),
            },
        )];
    };
    match gwt_core::coordination::load_snapshot(&tab.project_root) {
        Ok(snapshot) => {
            let entries = snapshot
                .board
                .entries
                .iter()
                .map(board_entry_view_from)
                .collect();
            vec![OutboundEvent::reply(
                client_id,
                gwt::BackendEvent::BoardSnapshot {
                    id: id.to_string(),
                    entries,
                },
            )]
        }
        Err(error) => vec![OutboundEvent::reply(
            client_id,
            gwt::BackendEvent::BoardError {
                id: id.to_string(),
                message: error.to_string(),
            },
        )],
    }
}

pub(super) fn resolve_window_agent_color(
    window: &gwt::PersistedWindowState,
) -> Option<gwt_agent::AgentColor> {
    let by_agent_id = window
        .agent_id
        .as_deref()
        .and_then(gwt_agent::resolve_agent_id);
    let agent_id = by_agent_id.or_else(|| window.preset.resolved_agent_id());
    agent_id.map(|id| id.default_color())
}

pub(super) fn board_entry_view_from(
    entry: &gwt_core::coordination::BoardEntry,
) -> gwt::BoardEntryView {
    let agent_color = entry
        .origin_agent_id
        .as_deref()
        .and_then(gwt_agent::resolve_agent_id)
        .map(|id| id.default_color());
    gwt::BoardEntryView {
        id: entry.id.clone(),
        author_kind: entry.author_kind.clone(),
        author: entry.author.clone(),
        kind: entry.kind.clone(),
        body: entry.body.clone(),
        created_at: entry.created_at,
        updated_at: entry.updated_at,
        origin_branch: entry.origin_branch.clone(),
        origin_agent_id: entry.origin_agent_id.clone(),
        agent_color,
    }
}

#[cfg(test)]
mod tests {
    use gwt::{PersistedWindowState, WindowGeometry, WindowPreset, WindowProcessStatus};
    use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};

    use super::{board_entry_view_from, resolve_window_agent_color};

    fn sample_window(preset: WindowPreset, status: WindowProcessStatus) -> PersistedWindowState {
        PersistedWindowState {
            id: "sample-1".to_string(),
            title: "Sample".to_string(),
            preset,
            geometry: WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 420.0,
            },
            z_index: 1,
            status,
            minimized: false,
            maximized: false,
            pre_maximize_geometry: None,
            persist: true,
            agent_id: None,
            agent_color: None,
        }
    }

    #[test]
    fn resolve_window_agent_color_prefers_explicit_agent_id() {
        let mut window = sample_window(WindowPreset::Agent, WindowProcessStatus::Running);
        window.agent_id = Some("gemini".into());
        assert_eq!(
            resolve_window_agent_color(&window),
            Some(gwt_agent::AgentColor::Magenta),
        );
    }

    #[test]
    fn resolve_window_agent_color_falls_back_to_preset() {
        let mut window = sample_window(WindowPreset::Claude, WindowProcessStatus::Running);
        window.agent_id = None;
        assert_eq!(
            resolve_window_agent_color(&window),
            Some(gwt_agent::AgentColor::Yellow),
        );
        let codex = sample_window(WindowPreset::Codex, WindowProcessStatus::Running);
        assert_eq!(
            resolve_window_agent_color(&codex),
            Some(gwt_agent::AgentColor::Cyan),
        );
    }

    #[test]
    fn resolve_window_agent_color_returns_none_for_agent_preset_without_id() {
        let window = sample_window(WindowPreset::Agent, WindowProcessStatus::Running);
        assert_eq!(resolve_window_agent_color(&window), None);
    }

    #[test]
    fn resolve_window_agent_color_handles_custom_agent_as_gray() {
        let mut window = sample_window(WindowPreset::Agent, WindowProcessStatus::Running);
        window.agent_id = Some("my-custom-agent".into());
        assert_eq!(
            resolve_window_agent_color(&window),
            Some(gwt_agent::AgentColor::Gray),
        );
    }

    #[test]
    fn board_entry_view_resolves_origin_agent_id_to_color() {
        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "Started task",
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_agent_id("codex");
        let view = board_entry_view_from(&entry);
        assert_eq!(view.agent_color, Some(gwt_agent::AgentColor::Cyan));
        assert_eq!(view.origin_agent_id.as_deref(), Some("codex"));
    }

    #[test]
    fn board_entry_view_returns_none_color_for_missing_origin_agent_id() {
        let entry = BoardEntry::new(
            AuthorKind::User,
            "akio",
            BoardEntryKind::Request,
            "Please look into X",
            None,
            None,
            vec![],
            vec![],
        );
        let view = board_entry_view_from(&entry);
        assert_eq!(view.agent_color, None);
        assert_eq!(view.origin_agent_id, None);
    }
}

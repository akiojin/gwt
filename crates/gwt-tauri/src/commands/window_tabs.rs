use crate::state::{AgentTabMenuState, AppState};
use gwt_core::StructuredError;
use serde::Deserialize;
use std::collections::HashSet;
use tauri::{Manager, State};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowAgentTabEntry {
    pub id: String,
    pub label: String,
    pub tab_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncWindowAgentTabsRequest {
    pub tabs: Vec<WindowAgentTabEntry>,
    pub active_tab_id: Option<String>,
}

fn normalize_active_tab_id(active_tab_id: Option<String>) -> Option<String> {
    active_tab_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
}

fn should_rebuild_window_menu(
    previous_tabs: &[AgentTabMenuState],
    next_tabs: &[AgentTabMenuState],
) -> bool {
    previous_tabs != next_tabs
}

fn normalize_tabs(entries: Vec<WindowAgentTabEntry>) -> Vec<AgentTabMenuState> {
    let mut seen = HashSet::new();
    let mut tabs = Vec::new();

    for entry in entries {
        let id = entry.id.trim().to_string();
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }

        let label = entry.label.trim().to_string();
        tabs.push(AgentTabMenuState {
            id,
            label: if label.is_empty() {
                "Agent".to_string()
            } else {
                label
            },
        });
    }

    tabs
}

#[tauri::command]
pub fn sync_window_agent_tabs(
    window: tauri::Window,
    state: State<AppState>,
    request: SyncWindowAgentTabsRequest,
) -> Result<(), StructuredError> {
    let tabs = normalize_tabs(request.tabs);
    let active_tab_id = normalize_active_tab_id(request.active_tab_id);
    let previous_tabs = state.window_agent_tabs_for_window(window.label()).tabs;

    state.set_window_agent_tabs(window.label(), tabs.clone(), active_tab_id);
    if !should_rebuild_window_menu(&previous_tabs, &tabs) {
        crate::menu::refresh_window_tab_checkmarks(window.app_handle(), &state)
            .map_err(|e| StructuredError::internal(&e.to_string(), "sync_window_agent_tabs"))?;
        return Ok(());
    }

    crate::menu::rebuild_menu(window.app_handle())
        .map_err(|e| StructuredError::internal(&e.to_string(), "sync_window_agent_tabs"))
}

#[tauri::command]
pub fn sync_window_active_tab(
    window: tauri::Window,
    state: State<AppState>,
    active_tab_id: Option<String>,
) -> Result<(), StructuredError> {
    let active_tab_id = normalize_active_tab_id(active_tab_id);
    state.set_window_agent_active_tab(window.label(), active_tab_id);
    crate::menu::refresh_window_tab_checkmarks(window.app_handle(), &state)
        .map_err(|e| StructuredError::internal(&e.to_string(), "sync_window_active_tab"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_tabs_drops_empty_and_duplicate_ids() {
        let tabs = normalize_tabs(vec![
            WindowAgentTabEntry {
                id: "".to_string(),
                label: "A".to_string(),
                tab_type: None,
            },
            WindowAgentTabEntry {
                id: "agent-1".to_string(),
                label: "One".to_string(),
                tab_type: None,
            },
            WindowAgentTabEntry {
                id: "agent-1".to_string(),
                label: "Duplicate".to_string(),
                tab_type: None,
            },
            WindowAgentTabEntry {
                id: " agent-2 ".to_string(),
                label: " ".to_string(),
                tab_type: Some("terminal".to_string()),
            },
        ]);

        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].id, "agent-1");
        assert_eq!(tabs[0].label, "One");
        assert_eq!(tabs[1].id, "agent-2");
        assert_eq!(tabs[1].label, "Agent");
    }

    #[test]
    fn normalize_active_tab_id_trims_and_drops_empty() {
        assert_eq!(
            normalize_active_tab_id(Some("  agent-1  ".to_string())),
            Some("agent-1".to_string())
        );
        assert_eq!(normalize_active_tab_id(Some("   ".to_string())), None);
        assert_eq!(normalize_active_tab_id(None), None);
    }

    #[test]
    fn should_rebuild_window_menu_only_when_tab_set_changes() {
        let previous = vec![AgentTabMenuState {
            id: "agent-1".to_string(),
            label: "one".to_string(),
        }];
        let same = vec![AgentTabMenuState {
            id: "agent-1".to_string(),
            label: "one".to_string(),
        }];
        let changed = vec![AgentTabMenuState {
            id: "agent-2".to_string(),
            label: "two".to_string(),
        }];

        assert!(!should_rebuild_window_menu(&previous, &same));
        assert!(should_rebuild_window_menu(&previous, &changed));
    }
}

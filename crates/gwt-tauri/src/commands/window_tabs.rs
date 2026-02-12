use crate::state::{AgentTabMenuState, AppState};
use serde::Deserialize;
use std::collections::HashSet;
use tauri::{Manager, State};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowAgentTabEntry {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncWindowAgentTabsRequest {
    pub tabs: Vec<WindowAgentTabEntry>,
    pub active_tab_id: Option<String>,
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
) -> Result<(), String> {
    let tabs = normalize_tabs(request.tabs);
    let active_tab_id = request.active_tab_id.map(|id| id.trim().to_string());

    state.set_window_agent_tabs(window.label(), tabs, active_tab_id);
    crate::menu::rebuild_menu(window.app_handle()).map_err(|e| e.to_string())
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
            },
            WindowAgentTabEntry {
                id: "agent-1".to_string(),
                label: "One".to_string(),
            },
            WindowAgentTabEntry {
                id: "agent-1".to_string(),
                label: "Duplicate".to_string(),
            },
            WindowAgentTabEntry {
                id: " agent-2 ".to_string(),
                label: " ".to_string(),
            },
        ]);

        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].id, "agent-1");
        assert_eq!(tabs[0].label, "One");
        assert_eq!(tabs[1].id, "agent-2");
        assert_eq!(tabs[1].label, "Agent");
    }
}

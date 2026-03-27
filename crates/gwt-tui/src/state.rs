//! TUI application state types

use std::time::Instant;

use gwt_core::terminal::AgentColor;

/// Prefix key state (e.g. Ctrl-B prefix for tmux-like bindings).
#[derive(Debug)]
pub enum PrefixState {
    Idle,
    Active(Instant),
}

/// Scroll position state.
#[derive(Debug, Clone, PartialEq)]
pub enum ScrollState {
    Live,
    Scrolled { offset: usize },
}

/// Application mode.
#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Management,
    ScrollMode,
}

/// Tab status for display.
#[derive(Debug, Clone, PartialEq)]
pub enum TabStatus {
    Running,
    Completed,
    Error,
}

/// Information about a single tab.
#[derive(Debug, Clone)]
pub struct TabInfo {
    pub pane_id: String,
    pub name: String,
    pub color: AgentColor,
    pub status: TabStatus,
    pub branch: String,
}

/// Top-level TUI state.
pub struct TuiState {
    pub tabs: Vec<TabInfo>,
    pub active_tab: usize,
    pub prefix_state: PrefixState,
    pub scroll_state: ScrollState,
    pub mode: AppMode,
}

impl TuiState {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,
            prefix_state: PrefixState::Idle,
            scroll_state: ScrollState::Live,
            mode: AppMode::Normal,
        }
    }

    /// Get the active tab's pane_id, if any.
    pub fn active_pane_id(&self) -> Option<&str> {
        self.tabs.get(self.active_tab).map(|t| t.pane_id.as_str())
    }

    /// Get scroll offset (0 = live tail).
    pub fn scroll_offset(&self) -> usize {
        match &self.scroll_state {
            ScrollState::Live => 0,
            ScrollState::Scrolled { offset } => *offset,
        }
    }
}

impl Default for TuiState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_state_new_defaults() {
        let state = TuiState::new();
        assert!(state.tabs.is_empty());
        assert_eq!(state.active_tab, 0);
        assert_eq!(state.scroll_state, ScrollState::Live);
        assert_eq!(state.mode, AppMode::Normal);
        assert!(state.active_pane_id().is_none());
        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn test_scroll_offset_live() {
        let state = TuiState::new();
        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn test_scroll_offset_scrolled() {
        let mut state = TuiState::new();
        state.scroll_state = ScrollState::Scrolled { offset: 42 };
        assert_eq!(state.scroll_offset(), 42);
    }

    #[test]
    fn test_active_pane_id_with_tabs() {
        let mut state = TuiState::new();
        state.tabs.push(TabInfo {
            pane_id: "pane-abc".to_string(),
            name: "shell".to_string(),
            color: AgentColor::White,
            status: TabStatus::Running,
            branch: "main".to_string(),
        });
        assert_eq!(state.active_pane_id(), Some("pane-abc"));
    }

    #[test]
    fn test_app_mode_variants() {
        assert_ne!(AppMode::Normal, AppMode::Management);
        assert_ne!(AppMode::Normal, AppMode::ScrollMode);
        assert_ne!(AppMode::Management, AppMode::ScrollMode);
    }
}

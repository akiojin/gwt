use std::collections::HashMap;
use std::time::Instant;

use gwt_core::terminal::AgentColor;

use crate::ui::management::ManagementState;
use crate::ui::split_layout::LayoutTree;

/// Prefix key state for Ctrl+G handling.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PrefixState {
    /// No prefix key active.
    #[default]
    Idle,
    /// Ctrl+G was pressed, waiting for next key.
    Active(Instant),
}

/// Scroll mode state.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ScrollState {
    /// Following live output.
    #[default]
    Live,
    /// Scrolled up by offset lines.
    Scrolled { offset: usize },
}

/// Application mode.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AppMode {
    /// Normal terminal interaction.
    #[default]
    Normal,
    /// Management panel visible.
    Management,
    /// Keyboard scroll mode active.
    ScrollMode,
    /// Launch agent dialog visible.
    LaunchDialog,
}

/// Type of tab content.
#[derive(Debug, Clone, PartialEq)]
pub enum TabType {
    /// Plain shell session.
    Shell,
    /// Coding agent (Claude Code, Codex, Gemini, etc.).
    Agent,
}

/// Pane status indicator.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneStatusIndicator {
    Running,
    Idle,
    Completed(i32),
    Error(String),
}

/// Information about a single tab (Window).
#[derive(Debug, Clone)]
pub struct TabInfo {
    pub pane_id: String,
    pub name: String,
    pub tab_type: TabType,
    pub color: AgentColor,
    pub status: PaneStatusIndicator,
    pub branch: Option<String>,
    pub spec_id: Option<String>,
    pub pane_count: usize,
}

/// Central TUI state.
#[derive(Debug)]
pub struct TuiState {
    pub tabs: Vec<TabInfo>,
    pub active_tab: usize,
    pub prefix_state: PrefixState,
    pub scroll_state: ScrollState,
    pub mode: AppMode,
    pub management: ManagementState,
    pub layout_trees: HashMap<String, LayoutTree>,
    pub zoomed: bool,
}

impl TuiState {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,
            prefix_state: PrefixState::default(),
            scroll_state: ScrollState::default(),
            mode: AppMode::default(),
            management: ManagementState::default(),
            layout_trees: HashMap::new(),
            zoomed: false,
        }
    }

    /// Get the focused pane_id for the active tab (respects split layout focus).
    pub fn focused_pane_id(&self) -> Option<String> {
        let tab = self.tabs.get(self.active_tab)?;
        if let Some(tree) = self.layout_trees.get(&tab.pane_id) {
            Some(tree.focused_pane().to_string())
        } else {
            Some(tab.pane_id.clone())
        }
    }

    /// Add a tab and set it as active.
    pub fn add_tab(&mut self, tab: TabInfo) {
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    /// Remove a tab by index. Returns the removed tab, if any.
    pub fn remove_tab(&mut self, index: usize) -> Option<TabInfo> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        Some(tab)
    }

    /// Switch to the next tab.
    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
        }
    }

    /// Switch to the previous tab.
    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab = if self.active_tab == 0 {
                self.tabs.len() - 1
            } else {
                self.active_tab - 1
            };
        }
    }

    /// Set the active tab by index (clamped to bounds).
    pub fn set_active_tab(&mut self, index: usize) {
        if !self.tabs.is_empty() {
            self.active_tab = index.min(self.tabs.len() - 1);
        }
    }

    /// Get the active tab info.
    pub fn active_tab_info(&self) -> Option<&TabInfo> {
        self.tabs.get(self.active_tab)
    }

    /// Number of tabs.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
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

    fn make_tab(name: &str) -> TabInfo {
        TabInfo {
            pane_id: format!("pane-{name}"),
            name: name.to_string(),
            tab_type: TabType::Shell,
            color: AgentColor::Green,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        }
    }

    #[test]
    fn test_new_state_is_empty() {
        let state = TuiState::new();
        assert!(state.tabs.is_empty());
        assert_eq!(state.active_tab, 0);
        assert_eq!(state.mode, AppMode::Normal);
    }

    #[test]
    fn test_add_tab_sets_active() {
        let mut state = TuiState::new();
        state.add_tab(make_tab("a"));
        assert_eq!(state.active_tab, 0);
        state.add_tab(make_tab("b"));
        assert_eq!(state.active_tab, 1);
    }

    #[test]
    fn test_remove_tab_clamps_active() {
        let mut state = TuiState::new();
        state.add_tab(make_tab("a"));
        state.add_tab(make_tab("b"));
        state.add_tab(make_tab("c"));
        state.active_tab = 2;
        state.remove_tab(2);
        assert_eq!(state.active_tab, 1);
    }

    #[test]
    fn test_remove_tab_out_of_bounds() {
        let mut state = TuiState::new();
        assert!(state.remove_tab(0).is_none());
    }

    #[test]
    fn test_remove_last_tab() {
        let mut state = TuiState::new();
        state.add_tab(make_tab("a"));
        state.remove_tab(0);
        assert!(state.tabs.is_empty());
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn test_next_tab_wraps() {
        let mut state = TuiState::new();
        state.add_tab(make_tab("a"));
        state.add_tab(make_tab("b"));
        state.active_tab = 1;
        state.next_tab();
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn test_prev_tab_wraps() {
        let mut state = TuiState::new();
        state.add_tab(make_tab("a"));
        state.add_tab(make_tab("b"));
        state.active_tab = 0;
        state.prev_tab();
        assert_eq!(state.active_tab, 1);
    }

    #[test]
    fn test_next_tab_empty() {
        let mut state = TuiState::new();
        state.next_tab();
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn test_set_active_tab_clamps() {
        let mut state = TuiState::new();
        state.add_tab(make_tab("a"));
        state.add_tab(make_tab("b"));
        state.set_active_tab(100);
        assert_eq!(state.active_tab, 1);
    }

    #[test]
    fn test_active_tab_info() {
        let mut state = TuiState::new();
        assert!(state.active_tab_info().is_none());
        state.add_tab(make_tab("test"));
        assert_eq!(state.active_tab_info().unwrap().name, "test");
    }
}

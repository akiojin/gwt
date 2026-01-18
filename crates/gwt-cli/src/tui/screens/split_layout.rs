//! Split layout for tmux multi-mode
//!
//! Provides a top-bottom split layout with branch list above and pane list below.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Focus state for the split layout
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusPanel {
    /// Branch list panel has focus
    #[default]
    BranchList,
    /// Pane list panel has focus
    PaneList,
}

impl FocusPanel {
    /// Toggle focus between panels
    pub fn toggle(&mut self) {
        *self = match self {
            FocusPanel::BranchList => FocusPanel::PaneList,
            FocusPanel::PaneList => FocusPanel::BranchList,
        };
    }

    /// Check if branch list has focus
    #[allow(dead_code)]
    pub fn is_branch_list(&self) -> bool {
        matches!(self, FocusPanel::BranchList)
    }

    /// Check if pane list has focus
    pub fn is_pane_list(&self) -> bool {
        matches!(self, FocusPanel::PaneList)
    }
}

/// State for the split layout
#[derive(Debug, Default)]
pub struct SplitLayoutState {
    /// Current focus panel
    pub focus: FocusPanel,
    /// Whether the pane list is visible (tmux mode only)
    pub pane_list_visible: bool,
    /// Height ratio for the branch list (0.0 - 1.0)
    pub branch_list_ratio: f32,
}

impl SplitLayoutState {
    /// Create a new split layout state
    pub fn new() -> Self {
        Self {
            focus: FocusPanel::BranchList,
            pane_list_visible: false,
            branch_list_ratio: 0.7, // 70% for branch list by default
        }
    }

    /// Enable tmux mode (show pane list)
    pub fn enable_tmux_mode(&mut self) {
        self.pane_list_visible = true;
    }

    /// Disable tmux mode (hide pane list)
    #[allow(dead_code)]
    pub fn disable_tmux_mode(&mut self) {
        self.pane_list_visible = false;
        self.focus = FocusPanel::BranchList;
    }

    /// Toggle focus between panels (Tab key)
    pub fn toggle_focus(&mut self) {
        if self.pane_list_visible {
            self.focus.toggle();
        }
    }

    /// Check if branch list has focus
    #[allow(dead_code)]
    pub fn branch_list_has_focus(&self) -> bool {
        self.focus.is_branch_list()
    }

    /// Check if pane list has focus
    pub fn pane_list_has_focus(&self) -> bool {
        self.pane_list_visible && self.focus.is_pane_list()
    }
}

/// Layout areas for the split view
#[derive(Debug, Clone)]
pub struct SplitLayoutAreas {
    /// Area for the branch list
    pub branch_list: Rect,
    /// Area for the pane list (may be zero height if not visible)
    pub pane_list: Rect,
}

/// Calculate the layout areas for the split view
pub fn calculate_split_layout(area: Rect, state: &SplitLayoutState) -> SplitLayoutAreas {
    if !state.pane_list_visible {
        // Single panel mode - branch list takes full area
        return SplitLayoutAreas {
            branch_list: area,
            pane_list: Rect::default(),
        };
    }

    // Split mode - calculate heights
    let branch_height = (area.height as f32 * state.branch_list_ratio) as u16;
    let pane_height = area.height.saturating_sub(branch_height);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(branch_height),
            Constraint::Length(pane_height),
        ])
        .split(area);

    SplitLayoutAreas {
        branch_list: chunks[0],
        pane_list: chunks[1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focus_panel_toggle() {
        let mut focus = FocusPanel::BranchList;
        focus.toggle();
        assert_eq!(focus, FocusPanel::PaneList);
        focus.toggle();
        assert_eq!(focus, FocusPanel::BranchList);
    }

    #[test]
    fn test_focus_panel_is_methods() {
        assert!(FocusPanel::BranchList.is_branch_list());
        assert!(!FocusPanel::BranchList.is_pane_list());
        assert!(FocusPanel::PaneList.is_pane_list());
        assert!(!FocusPanel::PaneList.is_branch_list());
    }

    #[test]
    fn test_split_layout_state_new() {
        let state = SplitLayoutState::new();
        assert!(!state.pane_list_visible);
        assert_eq!(state.focus, FocusPanel::BranchList);
        assert!((state.branch_list_ratio - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_split_layout_state_enable_tmux() {
        let mut state = SplitLayoutState::new();
        state.enable_tmux_mode();
        assert!(state.pane_list_visible);
    }

    #[test]
    fn test_split_layout_state_disable_tmux() {
        let mut state = SplitLayoutState::new();
        state.enable_tmux_mode();
        state.focus = FocusPanel::PaneList;
        state.disable_tmux_mode();
        assert!(!state.pane_list_visible);
        assert_eq!(state.focus, FocusPanel::BranchList);
    }

    #[test]
    fn test_split_layout_state_toggle_focus() {
        let mut state = SplitLayoutState::new();

        // Toggle without tmux mode should not change focus
        state.toggle_focus();
        assert!(state.branch_list_has_focus());

        // Enable tmux mode and toggle
        state.enable_tmux_mode();
        state.toggle_focus();
        assert!(state.pane_list_has_focus());
        state.toggle_focus();
        assert!(state.branch_list_has_focus());
    }

    #[test]
    fn test_split_layout_state_focus_methods() {
        let mut state = SplitLayoutState::new();
        state.enable_tmux_mode();

        assert!(state.branch_list_has_focus());
        assert!(!state.pane_list_has_focus());

        state.toggle_focus();
        assert!(!state.branch_list_has_focus());
        assert!(state.pane_list_has_focus());
    }

    #[test]
    fn test_calculate_split_layout_single_mode() {
        let area = Rect::new(0, 0, 80, 24);
        let state = SplitLayoutState::new();
        let layout = calculate_split_layout(area, &state);

        assert_eq!(layout.branch_list, area);
        assert_eq!(layout.pane_list.height, 0);
    }

    #[test]
    fn test_calculate_split_layout_split_mode() {
        let area = Rect::new(0, 0, 80, 20);
        let mut state = SplitLayoutState::new();
        state.enable_tmux_mode();
        state.branch_list_ratio = 0.5;

        let layout = calculate_split_layout(area, &state);

        assert_eq!(layout.branch_list.height, 10);
        assert_eq!(layout.pane_list.height, 10);
        assert_eq!(layout.branch_list.y, 0);
        assert_eq!(layout.pane_list.y, 10);
    }

    #[test]
    fn test_calculate_split_layout_ratio() {
        let area = Rect::new(0, 0, 80, 100);
        let mut state = SplitLayoutState::new();
        state.enable_tmux_mode();
        state.branch_list_ratio = 0.7;

        let layout = calculate_split_layout(area, &state);

        assert_eq!(layout.branch_list.height, 70);
        assert_eq!(layout.pane_list.height, 30);
    }
}

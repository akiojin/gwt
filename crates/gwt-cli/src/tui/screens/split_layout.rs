//! Layout for split view (branch list + terminal pane)
//!
//! Handles layout calculation for the main content area,
//! supporting branch-list-only, 50:50 split, and fullscreen terminal modes.

use ratatui::layout::{Constraint, Layout, Rect};

/// State for the split layout.
#[derive(Debug, Default)]
pub struct SplitLayoutState {
    /// Whether a terminal pane is active.
    pub has_terminal_pane: bool,
    /// Whether the terminal pane is in fullscreen mode.
    pub is_fullscreen: bool,
}

impl SplitLayoutState {
    /// Create a new layout state.
    /// Terminal pane is always visible (FR-046).
    pub fn new() -> Self {
        Self {
            has_terminal_pane: true,
            is_fullscreen: false,
        }
    }
}

/// Layout areas computed by `calculate_split_layout`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SplitLayoutAreas {
    /// Area for the branch list.
    pub branch_list: Rect,
    /// Area for the terminal pane (if active).
    pub terminal_pane: Option<Rect>,
}

/// Calculate the layout areas based on state and available space.
///
/// - No terminal pane: branch list takes full area.
/// - Fullscreen mode: terminal pane takes full area.
/// - Width < 80: fallback to terminal pane only (too narrow to split).
/// - Otherwise: 50:50 horizontal split.
pub fn calculate_split_layout(area: Rect, state: &SplitLayoutState) -> SplitLayoutAreas {
    if !state.has_terminal_pane {
        return SplitLayoutAreas {
            branch_list: area,
            terminal_pane: None,
        };
    }

    if state.is_fullscreen {
        return SplitLayoutAreas {
            branch_list: Rect::default(),
            terminal_pane: Some(area),
        };
    }

    if area.width < 80 {
        // Too narrow to split: fallback to terminal pane only
        return SplitLayoutAreas {
            branch_list: Rect::default(),
            terminal_pane: Some(area),
        };
    }

    // 50:50 horizontal split
    let chunks =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    SplitLayoutAreas {
        branch_list: chunks[0],
        terminal_pane: Some(chunks[1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_default() {
        let state = SplitLayoutState::new();
        // FR-046: Terminal pane is always visible
        assert!(state.has_terminal_pane);
        assert!(!state.is_fullscreen);
    }

    /// FR-046: Terminal pane is always shown even with new() default.
    #[test]
    fn test_always_shows_terminal_pane() {
        let state = SplitLayoutState::new();
        let area = Rect::new(0, 0, 160, 40);
        let layout = calculate_split_layout(area, &state);
        assert!(layout.terminal_pane.is_some());
    }

    #[test]
    fn test_no_terminal_pane_full_branch_list() {
        let area = Rect::new(0, 0, 120, 40);
        let state = SplitLayoutState {
            has_terminal_pane: false,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.branch_list, area);
        assert!(layout.terminal_pane.is_none());
    }

    #[test]
    fn test_terminal_pane_50_50_split() {
        let area = Rect::new(0, 0, 160, 40);
        let state = SplitLayoutState {
            has_terminal_pane: true,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.branch_list.width, 80);
        assert_eq!(layout.branch_list.height, 40);
        let tp = layout.terminal_pane.expect("terminal_pane should be Some");
        assert_eq!(tp.width, 80);
        assert_eq!(tp.height, 40);
    }

    #[test]
    fn test_fullscreen_mode_terminal_only() {
        let area = Rect::new(0, 0, 120, 40);
        let state = SplitLayoutState {
            has_terminal_pane: true,
            is_fullscreen: true,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.branch_list, Rect::default());
        let tp = layout.terminal_pane.expect("terminal_pane should be Some");
        assert_eq!(tp, area);
    }

    #[test]
    fn test_narrow_fallback_79_cols() {
        let area = Rect::new(0, 0, 79, 24);
        let state = SplitLayoutState {
            has_terminal_pane: true,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.branch_list, Rect::default());
        let tp = layout.terminal_pane.expect("terminal_pane should be Some");
        assert_eq!(tp, area);
    }

    #[test]
    fn test_80_cols_splits() {
        let area = Rect::new(0, 0, 80, 24);
        let state = SplitLayoutState {
            has_terminal_pane: true,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.branch_list.width, 40);
        let tp = layout.terminal_pane.expect("terminal_pane should be Some");
        assert_eq!(tp.width, 40);
    }

    #[test]
    fn test_160_cols_even_split() {
        let area = Rect::new(0, 0, 160, 40);
        let state = SplitLayoutState {
            has_terminal_pane: true,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.branch_list.width, 80);
        let tp = layout.terminal_pane.expect("terminal_pane should be Some");
        assert_eq!(tp.width, 80);
        assert_eq!(tp.x, 80);
    }
}

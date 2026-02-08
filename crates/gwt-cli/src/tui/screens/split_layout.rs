//! Layout for split view (left pane + agent pane)
//!
//! Handles layout calculation for the main screen area,
//! supporting left-pane-only, 50:50 split, and fullscreen agent pane modes.

use ratatui::layout::{Constraint, Layout, Rect};

/// State for the split layout.
#[derive(Debug, Default)]
pub struct SplitLayoutState {
    /// Whether an agent pane is active.
    pub has_agent_pane: bool,
    /// Whether the agent pane is in fullscreen mode.
    pub is_fullscreen: bool,
}

impl SplitLayoutState {
    /// Create a new layout state.
    /// Agent pane is always visible (FR-046).
    pub fn new() -> Self {
        Self {
            has_agent_pane: true,
            is_fullscreen: false,
        }
    }
}

/// Layout areas computed by `calculate_split_layout`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SplitLayoutAreas {
    /// Area for the left pane (screen-specific UI).
    pub left_pane: Rect,
    /// Area for the agent pane (if active).
    pub agent_pane: Option<Rect>,
}

/// Calculate the layout areas based on state and available space.
///
/// - No agent pane: left pane takes full area.
/// - Fullscreen mode: agent pane takes full area.
/// - Width < 80: fallback to agent pane only (too narrow to split).
/// - Otherwise: 50:50 horizontal split.
pub fn calculate_split_layout(area: Rect, state: &SplitLayoutState) -> SplitLayoutAreas {
    if !state.has_agent_pane {
        return SplitLayoutAreas {
            left_pane: area,
            agent_pane: None,
        };
    }

    if state.is_fullscreen {
        return SplitLayoutAreas {
            left_pane: Rect::default(),
            agent_pane: Some(area),
        };
    }

    if area.width < 80 {
        // Too narrow to split: fallback to agent pane only
        return SplitLayoutAreas {
            left_pane: Rect::default(),
            agent_pane: Some(area),
        };
    }

    // 50:50 horizontal split
    let chunks =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    SplitLayoutAreas {
        left_pane: chunks[0],
        agent_pane: Some(chunks[1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_default() {
        let state = SplitLayoutState::new();
        // FR-046: Agent pane is always visible
        assert!(state.has_agent_pane);
        assert!(!state.is_fullscreen);
    }

    /// FR-046: Agent pane is always shown even with new() default.
    #[test]
    fn test_always_shows_agent_pane() {
        let state = SplitLayoutState::new();
        let area = Rect::new(0, 0, 160, 40);
        let layout = calculate_split_layout(area, &state);
        assert!(layout.agent_pane.is_some());
    }

    #[test]
    fn test_no_agent_pane_full_left() {
        let area = Rect::new(0, 0, 120, 40);
        let state = SplitLayoutState {
            has_agent_pane: false,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.left_pane, area);
        assert!(layout.agent_pane.is_none());
    }

    #[test]
    fn test_agent_pane_50_50_split() {
        let area = Rect::new(0, 0, 160, 40);
        let state = SplitLayoutState {
            has_agent_pane: true,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.left_pane.width, 80);
        assert_eq!(layout.left_pane.height, 40);
        let tp = layout.agent_pane.expect("agent_pane should be Some");
        assert_eq!(tp.width, 80);
        assert_eq!(tp.height, 40);
    }

    #[test]
    fn test_fullscreen_mode_terminal_only() {
        let area = Rect::new(0, 0, 120, 40);
        let state = SplitLayoutState {
            has_agent_pane: true,
            is_fullscreen: true,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.left_pane, Rect::default());
        let tp = layout.agent_pane.expect("agent_pane should be Some");
        assert_eq!(tp, area);
    }

    #[test]
    fn test_narrow_fallback_79_cols() {
        let area = Rect::new(0, 0, 79, 24);
        let state = SplitLayoutState {
            has_agent_pane: true,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.left_pane, Rect::default());
        let tp = layout.agent_pane.expect("agent_pane should be Some");
        assert_eq!(tp, area);
    }

    #[test]
    fn test_80_cols_splits() {
        let area = Rect::new(0, 0, 80, 24);
        let state = SplitLayoutState {
            has_agent_pane: true,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.left_pane.width, 40);
        let tp = layout.agent_pane.expect("agent_pane should be Some");
        assert_eq!(tp.width, 40);
    }

    #[test]
    fn test_160_cols_even_split() {
        let area = Rect::new(0, 0, 160, 40);
        let state = SplitLayoutState {
            has_agent_pane: true,
            is_fullscreen: false,
        };
        let layout = calculate_split_layout(area, &state);
        assert_eq!(layout.left_pane.width, 80);
        let tp = layout.agent_pane.expect("agent_pane should be Some");
        assert_eq!(tp.width, 80);
        assert_eq!(tp.x, 80);
    }
}

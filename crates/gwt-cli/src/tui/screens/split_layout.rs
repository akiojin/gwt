//! Layout for tmux multi-mode
//!
//! Since PaneList is abolished, the branch list now takes full screen
//! with agent info integrated into each row.

use ratatui::layout::Rect;

/// State for the layout (simplified after PaneList removal)
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct SplitLayoutState {
    /// Spinner animation frame (deprecated - now in BranchListState)
    pub spinner_frame: usize,
}

impl SplitLayoutState {
    /// Create a new layout state
    pub fn new() -> Self {
        Self { spinner_frame: 0 }
    }

    /// Advance spinner to next frame (deprecated - now in BranchListState)
    #[allow(dead_code)]
    pub fn advance_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % 4;
    }

    /// Get current spinner character (deprecated - now in BranchListState)
    #[allow(dead_code)]
    pub fn spinner_char(&self) -> char {
        const SPINNER_FRAMES: [char; 4] = ['|', '/', '-', '\\'];
        SPINNER_FRAMES[self.spinner_frame]
    }
}

/// Layout areas (simplified - just branch list takes full area)
#[derive(Debug, Clone)]
pub struct SplitLayoutAreas {
    /// Area for the branch list (full screen)
    pub branch_list: Rect,
}

/// Calculate the layout areas
pub fn calculate_split_layout(area: Rect, _state: &SplitLayoutState) -> SplitLayoutAreas {
    // Branch list takes full area (no PaneList anymore)
    SplitLayoutAreas { branch_list: area }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_layout_state_new() {
        let state = SplitLayoutState::new();
        assert_eq!(state.spinner_frame, 0);
    }

    #[test]
    fn test_spinner_advance() {
        let mut state = SplitLayoutState::new();
        assert_eq!(state.spinner_char(), '|');
        state.advance_spinner();
        assert_eq!(state.spinner_char(), '/');
        state.advance_spinner();
        assert_eq!(state.spinner_char(), '-');
        state.advance_spinner();
        assert_eq!(state.spinner_char(), '\\');
        state.advance_spinner();
        assert_eq!(state.spinner_char(), '|'); // wraps around
    }

    #[test]
    fn test_calculate_split_layout_full_screen() {
        let area = Rect::new(0, 0, 80, 24);
        let state = SplitLayoutState::new();
        let layout = calculate_split_layout(area, &state);

        assert_eq!(layout.branch_list, area);
    }
}

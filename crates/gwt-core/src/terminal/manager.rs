//! Pane manager: manages multiple terminal panes
//!
//! Manages the lifecycle of up to 4 terminal panes,
//! including tab switching, fullscreen toggle, and batch operations.

use super::pane::{PaneConfig, TerminalPane};
use super::BuiltinLaunchConfig;
use super::TerminalError;

/// Maximum number of simultaneous panes (FR-033).
const DEFAULT_MAX_PANES: usize = 4;

/// Manages multiple terminal panes with tab-based switching.
pub struct PaneManager {
    panes: Vec<TerminalPane>,
    active_index: usize,
    max_panes: usize,
    is_fullscreen: bool,
}

impl PaneManager {
    /// Create a new PaneManager with max_panes=4.
    pub fn new() -> Self {
        Self {
            panes: Vec::new(),
            active_index: 0,
            max_panes: DEFAULT_MAX_PANES,
            is_fullscreen: false,
        }
    }

    /// Add a pane. Returns `PaneLimitReached` if at capacity (FR-034).
    pub fn add_pane(&mut self, pane: TerminalPane) -> Result<(), TerminalError> {
        if self.panes.len() >= self.max_panes {
            return Err(TerminalError::PaneLimitReached {
                max: self.max_panes,
            });
        }
        self.panes.push(pane);
        self.active_index = self.panes.len() - 1;
        Ok(())
    }

    /// Close and kill the pane at `index`. Returns the removed pane, or `None`.
    pub fn close_pane(&mut self, index: usize) -> Option<TerminalPane> {
        if index >= self.panes.len() {
            return None;
        }
        let mut pane = self.panes.remove(index);
        let _ = pane.kill();
        // Adjust active_index so it stays in bounds.
        if !self.panes.is_empty() {
            if self.active_index >= self.panes.len() {
                self.active_index = self.panes.len() - 1;
            }
        } else {
            self.active_index = 0;
        }
        Some(pane)
    }

    /// Close the currently active pane.
    pub fn close_active_pane(&mut self) -> Option<TerminalPane> {
        if self.panes.is_empty() {
            return None;
        }
        self.close_pane(self.active_index)
    }

    /// Switch to the next tab (wraps around).
    pub fn next_tab(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.active_index = (self.active_index + 1) % self.panes.len();
    }

    /// Switch to the previous tab (wraps around).
    pub fn prev_tab(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.active_index = (self.active_index + self.panes.len() - 1) % self.panes.len();
    }

    /// Get a reference to the active pane.
    pub fn active_pane(&self) -> Option<&TerminalPane> {
        self.panes.get(self.active_index)
    }

    /// Get a mutable reference to the active pane.
    pub fn active_pane_mut(&mut self) -> Option<&mut TerminalPane> {
        self.panes.get_mut(self.active_index)
    }

    /// Number of managed panes.
    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    /// True if no panes are managed.
    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    /// Toggle fullscreen mode.
    pub fn toggle_fullscreen(&mut self) {
        self.is_fullscreen = !self.is_fullscreen;
    }

    /// Whether fullscreen mode is active.
    pub fn is_fullscreen(&self) -> bool {
        self.is_fullscreen
    }

    /// Resize all panes.
    pub fn resize_all(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        for pane in &mut self.panes {
            pane.resize(rows, cols)?;
        }
        Ok(())
    }

    /// Kill all panes (FR-092).
    pub fn kill_all(&mut self) -> Result<(), TerminalError> {
        for pane in &mut self.panes {
            pane.kill()?;
        }
        Ok(())
    }

    /// Launch an agent in a new terminal pane.
    ///
    /// Creates a `TerminalPane` from the given config and adds it to the manager.
    /// Returns the generated pane ID, or `PaneLimitReached` if at capacity.
    pub fn launch_agent(
        &mut self,
        config: BuiltinLaunchConfig,
        rows: u16,
        cols: u16,
    ) -> Result<String, TerminalError> {
        let pane_id = format!(
            "pane-{}",
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("0")
        );
        let pane_config = PaneConfig {
            pane_id: pane_id.clone(),
            command: config.command,
            args: config.args,
            working_dir: config.working_dir,
            branch_name: config.branch_name,
            agent_name: config.agent_name,
            agent_color: config.agent_color,
            rows,
            cols,
            env_vars: config.env_vars,
        };
        let pane = TerminalPane::new(pane_config)?;
        self.add_pane(pane)?;
        Ok(pane_id)
    }

    /// Immutable slice of all panes.
    pub fn panes(&self) -> &[TerminalPane] {
        &self.panes
    }

    /// Current active index.
    pub fn active_index(&self) -> usize {
        self.active_index
    }
}

impl Default for PaneManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::pane::PaneConfig;
    use std::collections::HashMap;

    /// Helper: create a TerminalPane backed by `/usr/bin/true` (exits immediately).
    fn create_test_pane(id: &str) -> TerminalPane {
        TerminalPane::new(PaneConfig {
            pane_id: id.to_string(),
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "test-branch".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: ratatui::style::Color::Green,
            rows: 24,
            cols: 80,
            env_vars: HashMap::new(),
        })
        .expect("failed to create test pane")
    }

    // --- 1. Empty PaneManager ---

    #[test]
    fn test_empty_manager() {
        let mgr = PaneManager::new();
        assert!(mgr.is_empty());
        assert_eq!(mgr.pane_count(), 0);
        assert!(mgr.active_pane().is_none());
        assert_eq!(mgr.active_index(), 0);
        assert!(!mgr.is_fullscreen());
    }

    // --- 2. Add pane ---

    #[test]
    fn test_add_pane() {
        let mut mgr = PaneManager::new();
        let pane = create_test_pane("p1");
        mgr.add_pane(pane).unwrap();
        assert_eq!(mgr.pane_count(), 1);
        assert!(!mgr.is_empty());
        assert!(mgr.active_pane().is_some());
        assert_eq!(mgr.active_index(), 0);
    }

    // --- 3. Pane limit (4) ---

    #[test]
    fn test_pane_limit() {
        let mut mgr = PaneManager::new();
        for i in 0..4 {
            let pane = create_test_pane(&format!("p{i}"));
            mgr.add_pane(pane).unwrap();
        }
        assert_eq!(mgr.pane_count(), 4);

        // 5th pane should fail
        let pane5 = create_test_pane("p4");
        let result = mgr.add_pane(pane5);
        assert!(result.is_err());
        match result.unwrap_err() {
            TerminalError::PaneLimitReached { max } => assert_eq!(max, 4),
            other => panic!("Expected PaneLimitReached, got: {other:?}"),
        }
    }

    // --- 4. next_tab cycling ---

    #[test]
    fn test_next_tab() {
        let mut mgr = PaneManager::new();
        for i in 0..3 {
            mgr.add_pane(create_test_pane(&format!("p{i}"))).unwrap();
        }
        // After adding 3 panes, active_index is 2 (last added)
        assert_eq!(mgr.active_index(), 2);
        mgr.next_tab(); // 2 -> 0
        assert_eq!(mgr.active_index(), 0);
        mgr.next_tab(); // 0 -> 1
        assert_eq!(mgr.active_index(), 1);
        mgr.next_tab(); // 1 -> 2
        assert_eq!(mgr.active_index(), 2);
        mgr.next_tab(); // 2 -> 0 (wrap)
        assert_eq!(mgr.active_index(), 0);
    }

    // --- 5. prev_tab cycling ---

    #[test]
    fn test_prev_tab() {
        let mut mgr = PaneManager::new();
        for i in 0..3 {
            mgr.add_pane(create_test_pane(&format!("p{i}"))).unwrap();
        }
        // active_index is 2
        mgr.prev_tab(); // 2 -> 1
        assert_eq!(mgr.active_index(), 1);
        mgr.prev_tab(); // 1 -> 0
        assert_eq!(mgr.active_index(), 0);
        mgr.prev_tab(); // 0 -> 2 (wrap)
        assert_eq!(mgr.active_index(), 2);
    }

    // --- 6. close_pane adjusts active_index ---

    #[test]
    fn test_close_pane_middle() {
        let mut mgr = PaneManager::new();
        for i in 0..3 {
            mgr.add_pane(create_test_pane(&format!("p{i}"))).unwrap();
        }
        // active_index = 2. Close pane at index 1.
        let removed = mgr.close_pane(1);
        assert!(removed.is_some());
        assert_eq!(mgr.pane_count(), 2);
        // active_index should clamp: was 2, now max is 1
        assert!(mgr.active_index() <= 1);
    }

    // --- 7. close_active_pane ---

    #[test]
    fn test_close_active_pane() {
        let mut mgr = PaneManager::new();
        for i in 0..2 {
            mgr.add_pane(create_test_pane(&format!("p{i}"))).unwrap();
        }
        // active_index is 1
        let removed = mgr.close_active_pane();
        assert!(removed.is_some());
        assert_eq!(mgr.pane_count(), 1);
        assert_eq!(mgr.active_index(), 0);
    }

    // --- 8. Empty next_tab/prev_tab do not panic ---

    #[test]
    fn test_empty_tab_navigation() {
        let mut mgr = PaneManager::new();
        mgr.next_tab(); // should not panic
        mgr.prev_tab(); // should not panic
        assert_eq!(mgr.active_index(), 0);
    }

    // --- 9. Toggle fullscreen ---

    #[test]
    fn test_toggle_fullscreen() {
        let mut mgr = PaneManager::new();
        assert!(!mgr.is_fullscreen());
        mgr.toggle_fullscreen();
        assert!(mgr.is_fullscreen());
        mgr.toggle_fullscreen();
        assert!(!mgr.is_fullscreen());
    }

    // --- 10. Remove all panes ---

    #[test]
    fn test_remove_all_panes() {
        let mut mgr = PaneManager::new();
        for i in 0..3 {
            mgr.add_pane(create_test_pane(&format!("p{i}"))).unwrap();
        }
        while !mgr.is_empty() {
            mgr.close_pane(0);
        }
        assert!(mgr.is_empty());
        assert_eq!(mgr.pane_count(), 0);
    }

    // --- 11. active_pane_mut ---

    #[test]
    fn test_active_pane_mut() {
        let mut mgr = PaneManager::new();
        assert!(mgr.active_pane_mut().is_none());
        mgr.add_pane(create_test_pane("p0")).unwrap();
        assert!(mgr.active_pane_mut().is_some());
    }

    // --- 12. panes() returns slice ---

    #[test]
    fn test_panes_slice() {
        let mut mgr = PaneManager::new();
        assert!(mgr.panes().is_empty());
        mgr.add_pane(create_test_pane("p0")).unwrap();
        mgr.add_pane(create_test_pane("p1")).unwrap();
        assert_eq!(mgr.panes().len(), 2);
    }

    // --- 13. close_pane out-of-bounds ---

    #[test]
    fn test_close_pane_out_of_bounds() {
        let mut mgr = PaneManager::new();
        assert!(mgr.close_pane(0).is_none());
        assert!(mgr.close_pane(99).is_none());
    }

    // --- 14. close_active_pane on empty ---

    #[test]
    fn test_close_active_pane_empty() {
        let mut mgr = PaneManager::new();
        assert!(mgr.close_active_pane().is_none());
    }

    // --- 15. Default trait ---

    #[test]
    fn test_default() {
        let mgr = PaneManager::default();
        assert!(mgr.is_empty());
        assert!(!mgr.is_fullscreen());
    }

    // --- 16. add_pane sets active to newly added ---

    #[test]
    fn test_add_pane_sets_active_to_new() {
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        assert_eq!(mgr.active_index(), 0);
        mgr.add_pane(create_test_pane("p1")).unwrap();
        assert_eq!(mgr.active_index(), 1);
        mgr.add_pane(create_test_pane("p2")).unwrap();
        assert_eq!(mgr.active_index(), 2);
    }

    // --- 17. close last pane resets active_index ---

    #[test]
    fn test_close_last_pane_resets_index() {
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        mgr.close_pane(0);
        assert_eq!(mgr.active_index(), 0);
        assert!(mgr.is_empty());
    }

    // --- 18. resize_all ---

    #[test]
    fn test_resize_all() {
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        mgr.add_pane(create_test_pane("p1")).unwrap();
        // resize_all should not error (PTY may already be exited for /usr/bin/true,
        // but we still exercise the code path)
        let _ = mgr.resize_all(48, 120);
    }

    // --- 19. kill_all ---

    #[test]
    fn test_kill_all() {
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        mgr.add_pane(create_test_pane("p1")).unwrap();
        let _ = mgr.kill_all();
        // Panes should still be in the manager (kill does not remove them)
        assert_eq!(mgr.pane_count(), 2);
    }

    // --- 20. launch_agent creates pane and returns pane_id ---

    #[test]
    fn test_launch_agent_success() {
        use crate::terminal::BuiltinLaunchConfig;
        let mut mgr = PaneManager::new();
        let config = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/test".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: ratatui::style::Color::Cyan,
            env_vars: HashMap::new(),
        };
        let pane_id = mgr.launch_agent(config, 24, 80).unwrap();
        assert!(!pane_id.is_empty());
        assert!(pane_id.starts_with("pane-"));
        assert_eq!(mgr.pane_count(), 1);
    }

    // --- 21. launch_agent 4 times, 5th returns PaneLimitReached ---

    #[test]
    fn test_launch_agent_limit_reached() {
        use crate::terminal::BuiltinLaunchConfig;
        let mut mgr = PaneManager::new();
        for i in 0..4 {
            let config = BuiltinLaunchConfig {
                command: "/usr/bin/true".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: format!("feature/test-{i}"),
                agent_name: format!("agent-{i}"),
                agent_color: ratatui::style::Color::Green,
                env_vars: HashMap::new(),
            };
            mgr.launch_agent(config, 24, 80).unwrap();
        }
        assert_eq!(mgr.pane_count(), 4);

        // 5th should fail
        let config = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/test-4".to_string(),
            agent_name: "agent-4".to_string(),
            agent_color: ratatui::style::Color::Red,
            env_vars: HashMap::new(),
        };
        let result = mgr.launch_agent(config, 24, 80);
        assert!(result.is_err());
        match result.unwrap_err() {
            TerminalError::PaneLimitReached { max } => assert_eq!(max, 4),
            other => panic!("Expected PaneLimitReached, got: {other:?}"),
        }
    }

    // --- 22. launch_agent sets active_pane to new pane ---

    #[test]
    fn test_launch_agent_sets_active_pane() {
        use crate::terminal::BuiltinLaunchConfig;
        let mut mgr = PaneManager::new();
        let config1 = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/a".to_string(),
            agent_name: "agent-a".to_string(),
            agent_color: ratatui::style::Color::Green,
            env_vars: HashMap::new(),
        };
        let pane_id1 = mgr.launch_agent(config1, 24, 80).unwrap();

        let config2 = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/b".to_string(),
            agent_name: "agent-b".to_string(),
            agent_color: ratatui::style::Color::Blue,
            env_vars: HashMap::new(),
        };
        let pane_id2 = mgr.launch_agent(config2, 24, 80).unwrap();

        assert_ne!(pane_id1, pane_id2);
        // Active pane should be the latest one
        let active = mgr.active_pane().expect("should have active pane");
        assert_eq!(active.pane_id(), pane_id2);
    }
}

//! Pane manager: manages multiple terminal panes.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::pane::Pane;
use crate::TerminalError;

/// Configuration for launching an agent pane.
pub struct LaunchConfig {
    /// Command to execute.
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variables.
    pub env: HashMap<String, String>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
}

/// Manages multiple terminal panes.
pub struct PaneManager {
    panes: HashMap<String, Pane>,
    next_id: u64,
    cols: u16,
    rows: u16,
}

impl PaneManager {
    /// Create a new PaneManager with default terminal size.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            panes: HashMap::new(),
            next_id: 0,
            cols,
            rows,
        }
    }

    /// Generate a unique pane ID.
    fn next_pane_id(&mut self) -> String {
        let id = format!("pane-{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// Spawn a shell pane in the given working directory.
    ///
    /// Returns the pane ID.
    pub fn spawn_shell(
        &mut self,
        cwd: PathBuf,
        env: HashMap<String, String>,
    ) -> Result<String, TerminalError> {
        let id = self.next_pane_id();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let pane = Pane::new(
            id.clone(),
            shell,
            vec![],
            self.cols,
            self.rows,
            env,
            Some(cwd),
        )?;
        self.panes.insert(id.clone(), pane);
        Ok(id)
    }

    /// Launch an agent pane with the given configuration.
    ///
    /// Returns the pane ID.
    pub fn launch_agent(&mut self, config: LaunchConfig) -> Result<String, TerminalError> {
        let id = self.next_pane_id();
        let pane = Pane::new(
            id.clone(),
            config.command,
            config.args,
            self.cols,
            self.rows,
            config.env,
            config.cwd,
        )?;
        self.panes.insert(id.clone(), pane);
        Ok(id)
    }

    /// Close and kill a pane by ID.
    pub fn close_pane(&mut self, id: &str) -> Result<(), TerminalError> {
        let pane = self
            .panes
            .remove(id)
            .ok_or_else(|| TerminalError::PaneNotFound { id: id.to_string() })?;
        let _ = pane.kill();
        Ok(())
    }

    /// Get a reference to a pane by ID.
    pub fn get_pane(&self, id: &str) -> Option<&Pane> {
        self.panes.get(id)
    }

    /// Get a mutable reference to a pane by ID.
    pub fn get_pane_mut(&mut self, id: &str) -> Option<&mut Pane> {
        self.panes.get_mut(id)
    }

    /// Alias for `get_pane_mut` — used by gwt-tui.
    pub fn pane_mut_by_id(&mut self, id: &str) -> Option<&mut Pane> {
        self.panes.get_mut(id)
    }

    /// List all pane IDs.
    pub fn list_panes(&self) -> Vec<&str> {
        self.panes.keys().map(|s| s.as_str()).collect()
    }

    /// Number of active panes.
    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    /// Get references to all panes.
    pub fn panes(&self) -> Vec<&Pane> {
        self.panes.values().collect()
    }

    /// Get mutable references to all panes.
    pub fn panes_mut(&mut self) -> Vec<&mut Pane> {
        self.panes.values_mut().collect()
    }

    /// Resize all panes.
    pub fn resize_all(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        self.cols = cols;
        self.rows = rows;
        let mut errors = Vec::new();
        for (id, pane) in &mut self.panes {
            if let Err(e) = pane.resize(cols, rows) {
                errors.push(format!("{id}: {e}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(TerminalError::PtyIoError {
                details: format!("resize errors: {}", errors.join(", ")),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_manager_is_empty() {
        let mgr = PaneManager::new(80, 24);
        assert_eq!(mgr.pane_count(), 0);
        assert!(mgr.list_panes().is_empty());
    }

    #[test]
    fn test_spawn_shell() {
        let mut mgr = PaneManager::new(80, 24);
        let id = mgr
            .spawn_shell(std::env::temp_dir(), HashMap::new())
            .expect("spawn_shell failed");

        assert_eq!(mgr.pane_count(), 1);
        assert!(mgr.get_pane(&id).is_some());
        assert!(mgr.list_panes().contains(&id.as_str()));

        mgr.close_pane(&id).expect("close failed");
    }

    #[test]
    fn test_launch_agent() {
        let mut mgr = PaneManager::new(80, 24);
        let config = LaunchConfig {
            command: "/bin/echo".to_string(),
            args: vec!["agent-test".to_string()],
            env: HashMap::new(),
            cwd: None,
        };
        let id = mgr.launch_agent(config).expect("launch_agent failed");

        assert_eq!(mgr.pane_count(), 1);
        assert!(mgr.get_pane(&id).is_some());

        mgr.close_pane(&id).expect("close failed");
    }

    #[test]
    fn test_close_pane() {
        let mut mgr = PaneManager::new(80, 24);
        let id = mgr
            .spawn_shell(std::env::temp_dir(), HashMap::new())
            .expect("spawn failed");

        assert_eq!(mgr.pane_count(), 1);
        mgr.close_pane(&id).expect("close failed");
        assert_eq!(mgr.pane_count(), 0);
        assert!(mgr.get_pane(&id).is_none());
    }

    #[test]
    fn test_close_nonexistent_pane_returns_error() {
        let mut mgr = PaneManager::new(80, 24);
        let result = mgr.close_pane("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_panes() {
        let mut mgr = PaneManager::new(80, 24);
        let id1 = mgr
            .spawn_shell(std::env::temp_dir(), HashMap::new())
            .expect("spawn 1 failed");
        let id2 = mgr
            .spawn_shell(std::env::temp_dir(), HashMap::new())
            .expect("spawn 2 failed");

        assert_eq!(mgr.pane_count(), 2);
        assert_ne!(id1, id2);

        mgr.close_pane(&id1).expect("close 1 failed");
        mgr.close_pane(&id2).expect("close 2 failed");
    }

    #[test]
    fn test_get_pane_mut() {
        let mut mgr = PaneManager::new(80, 24);
        let id = mgr
            .spawn_shell(std::env::temp_dir(), HashMap::new())
            .expect("spawn failed");

        let pane = mgr.get_pane_mut(&id).expect("pane not found");
        pane.process_bytes(b"hello\r\n");
        let contents = pane.screen().contents();
        assert!(contents.contains("hello"));

        mgr.close_pane(&id).expect("close failed");
    }

    #[test]
    fn test_resize_all() {
        let mut mgr = PaneManager::new(80, 24);
        let id = mgr
            .spawn_shell(std::env::temp_dir(), HashMap::new())
            .expect("spawn failed");

        mgr.resize_all(120, 48).expect("resize_all failed");

        let pane = mgr.get_pane(&id).expect("pane not found");
        let screen = pane.screen();
        assert_eq!(screen.size(), (48, 120));

        mgr.close_pane(&id).expect("close failed");
    }

    #[test]
    fn test_unique_pane_ids() {
        let mut mgr = PaneManager::new(80, 24);
        let mut ids = Vec::new();
        for _ in 0..5 {
            let config = LaunchConfig {
                command: "/bin/echo".to_string(),
                args: vec!["test".to_string()],
                env: HashMap::new(),
                cwd: None,
            };
            ids.push(mgr.launch_agent(config).expect("launch failed"));
        }
        // All IDs should be unique
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());

        for id in &ids {
            mgr.close_pane(id).expect("close failed");
        }
    }
}

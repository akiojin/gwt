//! Pane manager: manages multiple terminal panes
//!
//! Manages the lifecycle of terminal panes,
//! including tab switching, fullscreen toggle, and batch operations.

use super::pane::{PaneConfig, TerminalPane};
use super::BuiltinLaunchConfig;
use super::TerminalError;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Manages multiple terminal panes with tab-based switching.
pub struct PaneManager {
    panes: Vec<TerminalPane>,
    active_index: usize,
    is_fullscreen: bool,
}

impl PaneManager {
    /// Create a new PaneManager.
    pub fn new() -> Self {
        Self {
            panes: Vec::new(),
            active_index: 0,
            is_fullscreen: false,
        }
    }

    /// Add a pane.
    pub fn add_pane(&mut self, pane: TerminalPane) -> Result<(), TerminalError> {
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
    /// Returns the generated pane ID.
    pub fn launch_agent(
        &mut self,
        repo_root: &Path,
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
        let branch_name_for_mapping = config.branch_name.clone();
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
        let _ = Self::save_branch_mapping(repo_root, &branch_name_for_mapping, &pane_id);
        Ok(pane_id)
    }

    /// Spawn a plain shell in a new terminal pane.
    ///
    /// Similar to `launch_agent()` but skips `save_branch_mapping()`.
    /// Returns the generated pane ID.
    pub fn spawn_shell(
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

    /// Get a mutable reference to a pane by its ID.
    pub fn pane_mut_by_id(&mut self, id: &str) -> Option<&mut TerminalPane> {
        self.panes.iter_mut().find(|p| p.pane_id() == id)
    }

    /// Immutable slice of all panes.
    pub fn panes(&self) -> &[TerminalPane] {
        &self.panes
    }

    /// Mutable slice of all panes.
    pub fn panes_mut(&mut self) -> &mut [TerminalPane] {
        &mut self.panes
    }

    /// Current active index.
    pub fn active_index(&self) -> usize {
        self.active_index
    }

    /// Find the index of a pane by branch name (FR-035).
    pub fn find_pane_index_by_branch(&self, branch_name: &str) -> Option<usize> {
        self.panes
            .iter()
            .position(|p| p.branch_name() == branch_name)
    }

    /// Set the active pane by index.
    pub fn set_active_index(&mut self, index: usize) {
        if index < self.panes.len() {
            self.active_index = index;
        }
    }

    /// Returns the directory containing branch→pane_id indices.
    fn index_dir() -> Result<PathBuf, TerminalError> {
        let home = dirs::home_dir().ok_or_else(|| TerminalError::ScrollbackError {
            details: "failed to determine home directory".to_string(),
        })?;
        Ok(home.join(".gwt").join("terminals").join("index"))
    }

    /// Returns the path to the branch→pane_id index file for a repository root.
    fn index_path_for_repo(repo_root: &Path) -> Result<PathBuf, TerminalError> {
        let dir = Self::index_dir()?;
        let repo_name = repo_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repo");

        // Match the TypeScript session filename approach:
        // Buffer.from(repositoryRoot).toString("base64").replace(/[/+=]/g, "_")
        let repo_path_str = repo_root.to_string_lossy();
        let hash = STANDARD.encode(repo_path_str.as_bytes());
        let hash_safe = hash.replace(['/', '+', '='], "_");

        Ok(dir.join(format!("{repo_name}_{hash_safe}.json")))
    }

    /// Saves a branch→pane_id mapping to `~/.gwt/terminals/index/{repoName}_{hash}.json`.
    pub fn save_branch_mapping(
        repo_root: &Path,
        branch: &str,
        pane_id: &str,
    ) -> Result<(), TerminalError> {
        Self::save_branch_mapping_to(Self::index_path_for_repo(repo_root)?, branch, pane_id)
    }

    /// Saves a branch→pane_id mapping to a specific path (for testing).
    pub fn save_branch_mapping_to(
        path: PathBuf,
        branch: &str,
        pane_id: &str,
    ) -> Result<(), TerminalError> {
        let mut map = Self::load_index(&path);
        map.insert(branch.to_string(), pane_id.to_string());
        Self::write_index(&path, &map)
    }

    /// Loads the pane_id for a given branch from the repository-scoped index file.
    pub fn load_pane_id_for_branch(repo_root: &Path, branch: &str) -> Option<String> {
        Self::load_pane_id_for_branch_from(Self::index_path_for_repo(repo_root).ok()?, branch)
    }

    /// Loads the pane_id for a given branch from a specific index file (for testing).
    pub fn load_pane_id_for_branch_from(path: PathBuf, branch: &str) -> Option<String> {
        let map = Self::load_index(&path);
        map.get(branch).cloned()
    }

    fn load_index(path: &PathBuf) -> HashMap<String, String> {
        fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    fn write_index(path: &PathBuf, map: &HashMap<String, String>) -> Result<(), TerminalError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to create directory: {e}"),
            })?;
        }
        let json =
            serde_json::to_string_pretty(map).map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to serialize index: {e}"),
            })?;
        fs::write(path, json).map_err(|e| TerminalError::ScrollbackError {
            details: format!("failed to write index file: {e}"),
        })
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
    use crate::terminal::AgentColor;
    use tempfile::TempDir;

    /// Helper: create a TerminalPane backed by `/usr/bin/true` (exits immediately).
    fn create_test_pane(id: &str) -> TerminalPane {
        TerminalPane::new(PaneConfig {
            pane_id: id.to_string(),
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "test-branch".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: AgentColor::Green,
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

    // --- 3. Add many panes ---

    #[test]
    fn test_add_many_panes() {
        let mut mgr = PaneManager::new();
        for i in 0..50 {
            let pane = create_test_pane(&format!("p{i}"));
            mgr.add_pane(pane).unwrap();
        }
        assert_eq!(mgr.pane_count(), 50);
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
        let repo_root = std::env::temp_dir();
        let config = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/test".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: AgentColor::Cyan,
            env_vars: HashMap::new(),
        };
        let pane_id = mgr.launch_agent(&repo_root, config, 24, 80).unwrap();
        assert!(!pane_id.is_empty());
        assert!(pane_id.starts_with("pane-"));
        assert_eq!(mgr.pane_count(), 1);
    }

    // --- 21. launch_agent can add many panes ---

    #[test]
    fn test_launch_agent_adds_many_panes() {
        use crate::terminal::BuiltinLaunchConfig;
        let mut mgr = PaneManager::new();
        let repo_root = std::env::temp_dir();
        for i in 0..10 {
            let config = BuiltinLaunchConfig {
                command: "/usr/bin/true".to_string(),
                args: vec![],
                working_dir: std::env::temp_dir(),
                branch_name: format!("feature/test-{i}"),
                agent_name: format!("agent-{i}"),
                agent_color: AgentColor::Green,
                env_vars: HashMap::new(),
            };
            mgr.launch_agent(&repo_root, config, 24, 80).unwrap();
        }
        assert_eq!(mgr.pane_count(), 10);
    }

    // --- 22. pane_mut_by_id ---

    #[test]
    fn test_pane_mut_by_id_found() {
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        mgr.add_pane(create_test_pane("p1")).unwrap();
        assert!(mgr.pane_mut_by_id("p0").is_some());
        assert!(mgr.pane_mut_by_id("p1").is_some());
    }

    #[test]
    fn test_pane_mut_by_id_not_found() {
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        assert!(mgr.pane_mut_by_id("nonexistent").is_none());
    }

    // --- 23. launch_agent sets active_pane to new pane ---

    #[test]
    fn test_launch_agent_sets_active_pane() {
        use crate::terminal::BuiltinLaunchConfig;
        let mut mgr = PaneManager::new();
        let repo_root = std::env::temp_dir();
        let config1 = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/a".to_string(),
            agent_name: "agent-a".to_string(),
            agent_color: AgentColor::Green,
            env_vars: HashMap::new(),
        };
        let pane_id1 = mgr.launch_agent(&repo_root, config1, 24, 80).unwrap();

        let config2 = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/b".to_string(),
            agent_name: "agent-b".to_string(),
            agent_color: AgentColor::Blue,
            env_vars: HashMap::new(),
        };
        let pane_id2 = mgr.launch_agent(&repo_root, config2, 24, 80).unwrap();

        assert_ne!(pane_id1, pane_id2);
        // Active pane should be the latest one
        let active = mgr.active_pane().expect("should have active pane");
        assert_eq!(active.pane_id(), pane_id2);
    }

    // --- 24. save_branch_mapping and load ---

    #[test]
    fn test_save_and_load_branch_mapping() {
        let tmp = TempDir::new().unwrap();
        let index_path = tmp.path().join("index.json");

        PaneManager::save_branch_mapping_to(index_path.clone(), "feature/foo", "pane-abc").unwrap();

        let result = PaneManager::load_pane_id_for_branch_from(index_path, "feature/foo");
        assert_eq!(result, Some("pane-abc".to_string()));
    }

    // --- 25. mapping overwrites on relaunch ---

    #[test]
    fn test_mapping_overwrites_on_relaunch() {
        let tmp = TempDir::new().unwrap();
        let index_path = tmp.path().join("index.json");

        PaneManager::save_branch_mapping_to(index_path.clone(), "feature/bar", "pane-old").unwrap();
        PaneManager::save_branch_mapping_to(index_path.clone(), "feature/bar", "pane-new").unwrap();

        let result = PaneManager::load_pane_id_for_branch_from(index_path, "feature/bar");
        assert_eq!(result, Some("pane-new".to_string()));
    }

    #[test]
    fn test_index_path_for_repo_is_scoped_and_safe() {
        let tmp = TempDir::new().unwrap();
        let repo_a = tmp.path().join("repo-a");
        let repo_b = tmp.path().join("repo-b");
        fs::create_dir_all(&repo_a).unwrap();
        fs::create_dir_all(&repo_b).unwrap();

        let path_a = PaneManager::index_path_for_repo(&repo_a).unwrap();
        let path_b = PaneManager::index_path_for_repo(&repo_b).unwrap();
        assert_ne!(path_a, path_b);

        // Ensure the file name is safe for common filesystems (no base64 punctuation).
        let name = path_a.file_name().unwrap().to_string_lossy();
        assert!(!name.contains('/'));
        assert!(!name.contains('+'));
        assert!(!name.contains('='));

        // Ensure indices live under ~/.gwt/terminals/index/.
        let dir = path_a.parent().unwrap();
        assert_eq!(dir.file_name().and_then(|s| s.to_str()), Some("index"));
    }

    // --- 26. load nonexistent branch returns None ---

    #[test]
    fn test_load_nonexistent_branch() {
        let tmp = TempDir::new().unwrap();
        let index_path = tmp.path().join("index.json");

        let result = PaneManager::load_pane_id_for_branch_from(index_path, "nonexistent");
        assert_eq!(result, None);
    }

    // --- 27. find_pane_index_by_branch (FR-035) ---

    #[test]
    fn test_find_pane_index_by_branch_found() {
        use crate::terminal::BuiltinLaunchConfig;
        let mut mgr = PaneManager::new();
        let repo_root = std::env::temp_dir();
        let config1 = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/alpha".to_string(),
            agent_name: "agent-a".to_string(),
            agent_color: AgentColor::Green,
            env_vars: HashMap::new(),
        };
        mgr.launch_agent(&repo_root, config1, 24, 80).unwrap();

        let config2 = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/beta".to_string(),
            agent_name: "agent-b".to_string(),
            agent_color: AgentColor::Blue,
            env_vars: HashMap::new(),
        };
        mgr.launch_agent(&repo_root, config2, 24, 80).unwrap();

        assert_eq!(mgr.find_pane_index_by_branch("feature/alpha"), Some(0));
        assert_eq!(mgr.find_pane_index_by_branch("feature/beta"), Some(1));
    }

    #[test]
    fn test_find_pane_index_by_branch_not_found() {
        let mgr = PaneManager::new();
        assert_eq!(mgr.find_pane_index_by_branch("nonexistent"), None);
    }

    // --- 28. set_active_index ---

    #[test]
    fn test_set_active_index() {
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        mgr.add_pane(create_test_pane("p1")).unwrap();
        mgr.add_pane(create_test_pane("p2")).unwrap();
        assert_eq!(mgr.active_index(), 2); // last added

        mgr.set_active_index(0);
        assert_eq!(mgr.active_index(), 0);

        mgr.set_active_index(1);
        assert_eq!(mgr.active_index(), 1);
    }

    #[test]
    fn test_set_active_index_out_of_bounds() {
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        assert_eq!(mgr.active_index(), 0);

        mgr.set_active_index(999); // out of bounds, should be ignored
        assert_eq!(mgr.active_index(), 0);
    }

    // --- 29. spawn_shell creates pane and returns pane_id ---

    #[test]
    fn test_spawn_shell_success() {
        use crate::terminal::BuiltinLaunchConfig;
        let mut mgr = PaneManager::new();
        let config = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec!["-l".to_string()],
            working_dir: std::env::temp_dir(),
            branch_name: "terminal".to_string(),
            agent_name: "terminal".to_string(),
            agent_color: AgentColor::White,
            env_vars: HashMap::new(),
        };
        let pane_id = mgr.spawn_shell(config, 24, 80).unwrap();
        assert!(!pane_id.is_empty());
        assert!(pane_id.starts_with("pane-"));
        assert_eq!(mgr.pane_count(), 1);
        assert_eq!(mgr.active_index(), 0);
    }

    // --- 30. spawn_shell sets active to new pane ---

    #[test]
    fn test_spawn_shell_sets_active() {
        use crate::terminal::BuiltinLaunchConfig;
        let mut mgr = PaneManager::new();
        mgr.add_pane(create_test_pane("p0")).unwrap();
        assert_eq!(mgr.active_index(), 0);

        let config = BuiltinLaunchConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "terminal".to_string(),
            agent_name: "terminal".to_string(),
            agent_color: AgentColor::White,
            env_vars: HashMap::new(),
        };
        let pane_id = mgr.spawn_shell(config, 24, 80).unwrap();
        assert_eq!(mgr.pane_count(), 2);
        assert_eq!(mgr.active_index(), 1);
        let active = mgr.active_pane().unwrap();
        assert_eq!(active.pane_id(), pane_id);
    }
}

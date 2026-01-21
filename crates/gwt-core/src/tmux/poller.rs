//! tmux pane state polling
//!
//! Provides polling functionality to monitor tmux pane states.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::pane::{list_panes, AgentPane, PaneInfo};

/// Message from the poller thread
#[derive(Debug, Clone)]
pub enum PollMessage {
    /// Pane list has been updated
    PanesUpdated(Vec<AgentPane>),
    /// A pane was closed
    PaneClosed(String),
    /// Polling error occurred
    Error(String),
}

/// Configuration for the poller
#[derive(Debug, Clone)]
pub struct PollerConfig {
    /// Session name to poll
    pub session: String,
    /// Polling interval
    pub interval: Duration,
}

impl Default for PollerConfig {
    fn default() -> Self {
        Self {
            session: String::new(),
            interval: Duration::from_secs(1),
        }
    }
}

/// Pane poller that runs in a background thread
pub struct PanePoller {
    /// Handle to the polling thread
    handle: Option<JoinHandle<()>>,
    /// Sender to stop the polling thread
    stop_tx: Option<Sender<()>>,
    /// Receiver for poll messages
    message_rx: Receiver<PollMessage>,
}

impl PanePoller {
    /// Start a new pane poller
    pub fn start(config: PollerConfig, agent_registry: AgentRegistry) -> Self {
        let (message_tx, message_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            poll_loop(config, agent_registry, message_tx, stop_rx);
        });

        Self {
            handle: Some(handle),
            stop_tx: Some(stop_tx),
            message_rx,
        }
    }

    /// Stop the poller
    pub fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    /// Try to receive a message without blocking
    pub fn try_recv(&self) -> Option<PollMessage> {
        self.message_rx.try_recv().ok()
    }

    /// Receive all pending messages
    pub fn drain_messages(&self) -> Vec<PollMessage> {
        let mut messages = Vec::new();
        while let Ok(msg) = self.message_rx.try_recv() {
            messages.push(msg);
        }
        messages
    }
}

impl Drop for PanePoller {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Registry to track agent panes
#[derive(Debug, Default, Clone)]
pub struct AgentRegistry {
    /// Map of pane_id to AgentPane
    panes: HashMap<String, AgentPane>,
}

impl AgentRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            panes: HashMap::new(),
        }
    }

    /// Register a new agent pane
    pub fn register(&mut self, pane: AgentPane) {
        self.panes.insert(pane.pane_id.clone(), pane);
    }

    /// Unregister a pane by ID
    pub fn unregister(&mut self, pane_id: &str) {
        self.panes.remove(pane_id);
    }

    /// Get all registered panes
    pub fn all_panes(&self) -> Vec<AgentPane> {
        self.panes.values().cloned().collect()
    }

    /// Get a pane by ID
    pub fn get(&self, pane_id: &str) -> Option<&AgentPane> {
        self.panes.get(pane_id)
    }

    /// Check if a pane is registered
    pub fn contains(&self, pane_id: &str) -> bool {
        self.panes.contains_key(pane_id)
    }

    /// Update pane information based on current tmux state
    pub fn update_from_pane_info(&mut self, pane_infos: &[PaneInfo]) -> Vec<String> {
        let current_ids: std::collections::HashSet<_> =
            pane_infos.iter().map(|p| p.pane_id.as_str()).collect();

        // Find closed panes
        let closed: Vec<String> = self
            .panes
            .keys()
            .filter(|id| !current_ids.contains(id.as_str()))
            .cloned()
            .collect();

        // Remove closed panes
        for id in &closed {
            self.panes.remove(id);
        }

        closed
    }

    /// Get the count of registered panes
    pub fn len(&self) -> usize {
        self.panes.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }
}

/// Main polling loop
fn poll_loop(
    config: PollerConfig,
    mut registry: AgentRegistry,
    tx: Sender<PollMessage>,
    stop_rx: Receiver<()>,
) {
    let mut last_poll = Instant::now();

    loop {
        // Check for stop signal
        if stop_rx.try_recv().is_ok() {
            break;
        }

        // Wait for next poll interval
        let elapsed = last_poll.elapsed();
        if elapsed < config.interval {
            thread::sleep(config.interval - elapsed);
        }
        last_poll = Instant::now();

        // Poll pane states
        match list_panes(&config.session) {
            Ok(pane_infos) => {
                // Check for closed panes
                let closed = registry.update_from_pane_info(&pane_infos);
                for pane_id in closed {
                    if tx.send(PollMessage::PaneClosed(pane_id)).is_err() {
                        return;
                    }
                }

                // Send updated pane list
                let panes = registry.all_panes();
                if tx.send(PollMessage::PanesUpdated(panes)).is_err() {
                    return;
                }
            }
            Err(e) => {
                if tx.send(PollMessage::Error(e.to_string())).is_err() {
                    return;
                }
            }
        }
    }
}

/// Calculate diff between two pane lists
pub fn diff_pane_lists(old: &[AgentPane], new: &[AgentPane]) -> PaneDiff {
    let old_ids: std::collections::HashSet<_> = old.iter().map(|p| &p.pane_id).collect();
    let new_ids: std::collections::HashSet<_> = new.iter().map(|p| &p.pane_id).collect();

    let added: Vec<_> = new
        .iter()
        .filter(|p| !old_ids.contains(&p.pane_id))
        .cloned()
        .collect();

    let removed: Vec<_> = old
        .iter()
        .filter(|p| !new_ids.contains(&p.pane_id))
        .map(|p| p.pane_id.clone())
        .collect();

    PaneDiff { added, removed }
}

/// Difference between two pane lists
#[derive(Debug, Default)]
pub struct PaneDiff {
    /// Newly added panes
    pub added: Vec<AgentPane>,
    /// Removed pane IDs
    pub removed: Vec<String>,
}

impl PaneDiff {
    /// Check if there are any changes
    pub fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn create_test_pane(id: &str, branch: &str) -> AgentPane {
        AgentPane::new(
            id.to_string(),
            branch.to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        )
    }

    #[test]
    fn test_poller_config_default() {
        let config = PollerConfig::default();
        assert!(config.session.is_empty());
        assert_eq!(config.interval, Duration::from_secs(1));
    }

    #[test]
    fn test_agent_registry_new() {
        let registry = AgentRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_agent_registry_register() {
        let mut registry = AgentRegistry::new();
        let pane = create_test_pane("1", "feature/a");
        registry.register(pane);

        assert_eq!(registry.len(), 1);
        assert!(registry.contains("1"));
    }

    #[test]
    fn test_agent_registry_unregister() {
        let mut registry = AgentRegistry::new();
        registry.register(create_test_pane("1", "feature/a"));
        registry.register(create_test_pane("2", "feature/b"));

        registry.unregister("1");
        assert_eq!(registry.len(), 1);
        assert!(!registry.contains("1"));
        assert!(registry.contains("2"));
    }

    #[test]
    fn test_agent_registry_all_panes() {
        let mut registry = AgentRegistry::new();
        registry.register(create_test_pane("1", "feature/a"));
        registry.register(create_test_pane("2", "feature/b"));

        let panes = registry.all_panes();
        assert_eq!(panes.len(), 2);
    }

    #[test]
    fn test_agent_registry_update_from_pane_info() {
        let mut registry = AgentRegistry::new();
        registry.register(create_test_pane("1", "feature/a"));
        registry.register(create_test_pane("2", "feature/b"));

        // Simulate pane 1 still running, pane 2 closed
        let pane_infos = vec![PaneInfo {
            pane_id: "1".to_string(),
            pane_pid: 12345,
            current_command: "claude".to_string(),
            current_path: None,
        }];

        let closed = registry.update_from_pane_info(&pane_infos);
        assert_eq!(closed, vec!["2"]);
        assert_eq!(registry.len(), 1);
        assert!(registry.contains("1"));
        assert!(!registry.contains("2"));
    }

    #[test]
    fn test_diff_pane_lists_added() {
        let old = vec![create_test_pane("1", "a")];
        let new = vec![create_test_pane("1", "a"), create_test_pane("2", "b")];

        let diff = diff_pane_lists(&old, &new);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].pane_id, "2");
        assert!(diff.removed.is_empty());
        assert!(diff.has_changes());
    }

    #[test]
    fn test_diff_pane_lists_removed() {
        let old = vec![create_test_pane("1", "a"), create_test_pane("2", "b")];
        let new = vec![create_test_pane("1", "a")];

        let diff = diff_pane_lists(&old, &new);
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed, vec!["2"]);
        assert!(diff.has_changes());
    }

    #[test]
    fn test_diff_pane_lists_no_changes() {
        let old = vec![create_test_pane("1", "a")];
        let new = vec![create_test_pane("1", "a")];

        let diff = diff_pane_lists(&old, &new);
        assert!(!diff.has_changes());
    }
}

#![allow(dead_code)]
//! Assistant Mode monitor — polls pane and git state, emits change events.

use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

use crate::state::AppState;

const POLL_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
pub struct PaneSnapshot {
    pub pane_id: String,
    pub agent_name: String,
    pub status: String,
    pub scrollback_hash: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusSnapshot {
    pub branch: String,
    pub uncommitted_count: u32,
    pub unpushed_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonitorSnapshot {
    pub panes: Vec<PaneSnapshot>,
    pub git: GitStatusSnapshot,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub enum MonitorEvent {
    SnapshotChanged(MonitorSnapshot),
}

pub struct ChangeDetector {
    prev_snapshot: Option<MonitorSnapshot>,
}

impl ChangeDetector {
    pub fn new() -> Self {
        Self {
            prev_snapshot: None,
        }
    }

    pub fn detect_change(&mut self, snapshot: &MonitorSnapshot) -> bool {
        let changed = match &self.prev_snapshot {
            None => true,
            Some(prev) => !snapshots_equal(prev, snapshot),
        };
        if changed {
            self.prev_snapshot = Some(snapshot.clone());
        }
        changed
    }
}

fn snapshots_equal(a: &MonitorSnapshot, b: &MonitorSnapshot) -> bool {
    if a.panes.len() != b.panes.len() {
        return false;
    }
    for (pa, pb) in a.panes.iter().zip(b.panes.iter()) {
        if pa.pane_id != pb.pane_id
            || pa.status != pb.status
            || pa.scrollback_hash != pb.scrollback_hash
        {
            return false;
        }
    }
    a.git.branch == b.git.branch
        && a.git.uncommitted_count == b.git.uncommitted_count
        && a.git.unpushed_count == b.git.unpushed_count
}

/// Handle for stopping the monitor task.
pub struct AssistantMonitorHandle {
    stop_tx: mpsc::Sender<()>,
}

impl AssistantMonitorHandle {
    pub async fn stop(&self) {
        let _ = self.stop_tx.send(()).await;
    }
}

/// Start the assistant monitor polling loop.
pub fn start_monitor(
    state: &AppState,
    event_tx: mpsc::Sender<MonitorEvent>,
) -> AssistantMonitorHandle {
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);

    // Clone what we need from state for the spawned task
    // AppState is not Send, so the monitor task will be integrated
    // with Tauri's state management in Phase 5. For now, spawn a
    // polling task that emits default snapshots.
    let _ = state;
    tokio::spawn(async move {
        let mut detector = ChangeDetector::new();
        let mut interval = tokio::time::interval(POLL_INTERVAL);

        loop {
            tokio::select! {
                _ = stop_rx.recv() => {
                    break;
                }
                _ = interval.tick() => {
                    // In the actual integration, this will capture pane state
                    // and git status from AppState. For now, emit a default snapshot.
                    let snapshot = MonitorSnapshot {
                        panes: Vec::new(),
                        git: GitStatusSnapshot {
                            branch: String::new(),
                            uncommitted_count: 0,
                            unpushed_count: 0,
                        },
                        timestamp: chrono::Utc::now().timestamp(),
                    };

                    if detector.detect_change(&snapshot)
                        && event_tx
                            .send(MonitorEvent::SnapshotChanged(snapshot))
                            .await
                            .is_err()
                    {
                        warn!("Monitor event receiver dropped; stopping monitor");
                        break;
                    }
                }
            }
        }
    });

    AssistantMonitorHandle { stop_tx }
}

/// Hash scrollback text for change detection.
pub fn hash_scrollback(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_change_detector_first_is_always_changed() {
        let mut detector = ChangeDetector::new();
        let snapshot = MonitorSnapshot {
            panes: Vec::new(),
            git: GitStatusSnapshot {
                branch: "main".to_string(),
                uncommitted_count: 0,
                unpushed_count: 0,
            },
            timestamp: 0,
        };
        assert!(detector.detect_change(&snapshot));
    }

    #[test]
    fn test_change_detector_same_snapshot_no_change() {
        let mut detector = ChangeDetector::new();
        let snapshot = MonitorSnapshot {
            panes: Vec::new(),
            git: GitStatusSnapshot {
                branch: "main".to_string(),
                uncommitted_count: 0,
                unpushed_count: 0,
            },
            timestamp: 0,
        };
        detector.detect_change(&snapshot);
        // Same content, different timestamp — should not trigger
        let snapshot2 = MonitorSnapshot {
            panes: Vec::new(),
            git: GitStatusSnapshot {
                branch: "main".to_string(),
                uncommitted_count: 0,
                unpushed_count: 0,
            },
            timestamp: 100,
        };
        assert!(!detector.detect_change(&snapshot2));
    }

    #[test]
    fn test_hash_scrollback() {
        let h1 = hash_scrollback("hello");
        let h2 = hash_scrollback("hello");
        let h3 = hash_scrollback("world");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }
}

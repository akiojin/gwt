#![allow(dead_code)]
//! Assistant Mode monitor: polls pane and git state, emits change events.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::Duration,
};

use gwt_core::{
    git::{self, Branch},
    terminal::pane::PaneStatus,
};
use serde::Serialize;
use tauri::Manager;
use tokio::sync::mpsc;
use tracing::warn;

use crate::{commands::project::resolve_repo_path_for_project_root, state::AppState};

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const SCROLLBACK_HASH_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PaneSnapshot {
    pub pane_id: String,
    pub agent_name: String,
    pub branch: String,
    pub status: String,
    pub scrollback_hash: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GitStatusSnapshot {
    pub branch: String,
    pub uncommitted_count: u32,
    pub unpushed_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonitorSnapshot {
    pub panes: Vec<PaneSnapshot>,
    pub git: GitStatusSnapshot,
    pub pending_consultations: u32,
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
    a.panes == b.panes && a.git == b.git && a.pending_consultations == b.pending_consultations
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
    app_handle: tauri::AppHandle,
    window_label: String,
    project_root: String,
    event_tx: mpsc::Sender<MonitorEvent>,
) -> AssistantMonitorHandle {
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);

    tokio::spawn(async move {
        let mut detector = ChangeDetector::new();
        let mut interval = tokio::time::interval(POLL_INTERVAL);

        loop {
            tokio::select! {
                _ = stop_rx.recv() => {
                    break;
                }
                _ = interval.tick() => {
                    let ah = app_handle.clone();
                    let wl = window_label.clone();
                    let pr = project_root.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        collect_snapshot(&ah, &wl, &pr)
                    }).await;
                    match result {
                        Ok(Ok(snapshot)) => {
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
                        Ok(Err(err)) => {
                            warn!(window = %window_label, error = %err, "Failed to collect assistant monitor snapshot");
                        }
                        Err(join_err) => {
                            warn!(window = %window_label, error = %join_err, "collect_snapshot panicked");
                        }
                    }
                }
            }
        }
    });

    AssistantMonitorHandle { stop_tx }
}

fn collect_snapshot(
    app_handle: &tauri::AppHandle,
    _window_label: &str,
    project_root: &str,
) -> Result<MonitorSnapshot, String> {
    let state = app_handle.state::<AppState>();
    let repo_path = resolve_repo_path_for_project_root(Path::new(project_root))
        .map_err(|e| format!("Failed to resolve repository path: {e}"))?;
    let current_branch = Branch::current(&repo_path)
        .map_err(|e| format!("Failed to resolve current branch: {e}"))?;
    let worktree_path = resolve_worktree_path(&repo_path, current_branch.as_ref())
        .unwrap_or_else(|| repo_path.clone());

    let branch = current_branch
        .as_ref()
        .map(|value| value.name.clone())
        .unwrap_or_default();
    let unpushed_count = current_branch
        .as_ref()
        .map(|value| value.ahead.min(u32::MAX as usize) as u32)
        .unwrap_or(0);
    let uncommitted_count = git::get_working_tree_status(&worktree_path)
        .map(|entries| entries.len().min(u32::MAX as usize) as u32)
        .unwrap_or(0);

    let panes = collect_project_panes(&state, &repo_path)?;
    let pending_consultations =
        crate::consultation::count_pending_consultations(Path::new(project_root));

    Ok(MonitorSnapshot {
        panes,
        git: GitStatusSnapshot {
            branch,
            uncommitted_count,
            unpushed_count,
        },
        pending_consultations,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

fn collect_project_panes(state: &AppState, repo_path: &Path) -> Result<Vec<PaneSnapshot>, String> {
    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {e}"))?;
    let panes = manager
        .panes_mut()
        .iter_mut()
        .filter(|pane| pane.project_root() == repo_path)
        .map(|pane| {
            let _ = pane.check_status();
            let status = match pane.status() {
                PaneStatus::Running => "running".to_string(),
                PaneStatus::Completed(code) => format!("completed({code})"),
                PaneStatus::Error(message) => format!("error: {message}"),
            };
            let scrollback_hash = pane
                .read_scrollback_tail_raw(SCROLLBACK_HASH_BYTES)
                .map(|bytes| hash_scrollback_bytes(&bytes))
                .unwrap_or(0);
            PaneSnapshot {
                pane_id: pane.pane_id().to_string(),
                agent_name: pane.agent_name().to_string(),
                branch: pane.branch_name().to_string(),
                status,
                scrollback_hash,
            }
        })
        .collect::<Vec<_>>();
    Ok(panes)
}

fn resolve_worktree_path(repo_path: &Path, current_branch: Option<&Branch>) -> Option<PathBuf> {
    if !git::is_bare_repository(repo_path) {
        return Some(repo_path.to_path_buf());
    }

    let manager = gwt_core::worktree::WorktreeManager::new(repo_path).ok()?;
    let worktrees = manager.list_basic().ok()?;

    if let Some(branch_name) = current_branch.map(|branch| branch.name.as_str()) {
        if let Some(worktree) = worktrees.iter().find(|worktree| {
            worktree.is_active() && worktree.branch.as_deref() == Some(branch_name)
        }) {
            return Some(worktree.path.clone());
        }
    }

    worktrees
        .iter()
        .find(|worktree| worktree.is_active() && !worktree.is_main)
        .map(|worktree| worktree.path.clone())
}

fn hash_scrollback_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
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
            pending_consultations: 0,
            timestamp: 0,
        };
        assert!(detector.detect_change(&snapshot));
    }

    #[test]
    fn test_change_detector_same_snapshot_no_change() {
        let mut detector = ChangeDetector::new();
        let snapshot = MonitorSnapshot {
            panes: vec![PaneSnapshot {
                pane_id: "pane-1".to_string(),
                agent_name: "codex".to_string(),
                branch: "feature/x".to_string(),
                status: "running".to_string(),
                scrollback_hash: 1,
            }],
            git: GitStatusSnapshot {
                branch: "main".to_string(),
                uncommitted_count: 0,
                unpushed_count: 0,
            },
            pending_consultations: 0,
            timestamp: 0,
        };
        detector.detect_change(&snapshot);
        let snapshot2 = MonitorSnapshot {
            panes: snapshot.panes.clone(),
            git: snapshot.git.clone(),
            pending_consultations: 0,
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

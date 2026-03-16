#![allow(dead_code)]
//! Assistant Mode monitor: polls pane and git state, emits change events.

use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::commands::terminal::capture_scrollback_tail_from_state;
use crate::state::AppState;
use gwt_core::git::{self, Branch};
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::worktree::WorktreeManager;
use tauri::Manager;

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const MONITOR_SCROLLBACK_BYTES: usize = 4096;

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

    pub fn stop_now(&self) {
        let _ = self.stop_tx.try_send(());
    }
}

pub(crate) fn resolve_project_repo_path(
    state: &AppState,
    window_label: &str,
) -> Result<Option<PathBuf>, String> {
    let Some(project_path) = state.project_for_window(window_label) else {
        return Ok(None);
    };

    resolve_repo_path_for_project_root(Path::new(&project_path))
        .map(Some)
        .map_err(|e| format!("Failed to resolve repository path: {}", e))
}

fn pane_status_label(status: &PaneStatus) -> String {
    match status {
        PaneStatus::Running => "running".to_string(),
        PaneStatus::Completed(code) => format!("completed({})", code),
        PaneStatus::Error(message) => format!("error: {}", message),
    }
}

fn collect_pane_snapshots(
    state: &AppState,
    repo_path: Option<&Path>,
) -> Result<Vec<PaneSnapshot>, String> {
    let Some(repo_path) = repo_path else {
        return Ok(Vec::new());
    };

    let pane_meta = {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        manager
            .panes_mut()
            .iter_mut()
            .filter(|pane| pane.project_root() == repo_path)
            .map(|pane| {
                let _ = pane.check_status();
                (
                    pane.pane_id().to_string(),
                    pane.agent_name().to_string(),
                    pane_status_label(pane.status()),
                )
            })
            .collect::<Vec<_>>()
    };

    let mut panes = Vec::with_capacity(pane_meta.len());
    for (pane_id, agent_name, status) in pane_meta {
        let scrollback = capture_scrollback_tail_from_state(
            state,
            &pane_id,
            MONITOR_SCROLLBACK_BYTES,
            Some(repo_path),
        )
        .unwrap_or_default();
        panes.push(PaneSnapshot {
            pane_id,
            agent_name,
            status,
            scrollback_hash: hash_scrollback(&scrollback),
        });
    }

    Ok(panes)
}

fn resolve_worktree_path(repo_path: &Path, current_branch: Option<&Branch>) -> Option<PathBuf> {
    if !git::is_bare_repository(repo_path) {
        return Some(repo_path.to_path_buf());
    }

    let manager = WorktreeManager::new(repo_path).ok()?;
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

fn build_git_snapshot(repo_path: Option<&Path>) -> Result<GitStatusSnapshot, String> {
    let Some(repo_path) = repo_path else {
        return Ok(GitStatusSnapshot {
            branch: String::new(),
            uncommitted_count: 0,
            unpushed_count: 0,
        });
    };

    let current_branch = Branch::current(repo_path)
        .map_err(|e| format!("Failed to resolve current branch: {}", e))?;

    let (branch, unpushed_count) = current_branch
        .as_ref()
        .map(|branch| {
            (
                branch.name.clone(),
                branch.ahead.min(u32::MAX as usize) as u32,
            )
        })
        .unwrap_or_else(|| (String::new(), 0));

    let uncommitted_count = resolve_worktree_path(repo_path, current_branch.as_ref())
        .and_then(|path| git::get_working_tree_status(&path).ok())
        .map(|entries| entries.len().min(u32::MAX as usize) as u32)
        .unwrap_or(0);

    Ok(GitStatusSnapshot {
        branch,
        uncommitted_count,
        unpushed_count,
    })
}

pub(crate) fn build_snapshot_for_window(
    state: &AppState,
    window_label: &str,
) -> Result<MonitorSnapshot, String> {
    let repo_path = resolve_project_repo_path(state, window_label)?;
    Ok(MonitorSnapshot {
        panes: collect_pane_snapshots(state, repo_path.as_deref())?,
        git: build_git_snapshot(repo_path.as_deref())?,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

/// Start the assistant monitor polling loop.
pub fn start_monitor(
    app_handle: tauri::AppHandle<tauri::Wry>,
    window_label: String,
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
                    let state = app_handle.state::<AppState>();
                    let snapshot = match build_snapshot_for_window(&state, &window_label) {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            warn!(%error, %window_label, "Failed to build assistant monitor snapshot");
                            continue;
                        }
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

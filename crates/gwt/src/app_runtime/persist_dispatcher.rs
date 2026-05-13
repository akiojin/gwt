//! Async writer that owns the disk I/O for `AppRuntime::persist`.
//!
//! Issue #2694 Phase B: the previous `persist()` path ran `std::fs::write`
//! inline on the tao event loop thread, so each window event (focus, resize,
//! arrange, viewport pan, ...) blocked the UI for as long as Windows Defender
//! / EDR scans held the session-state file. `PersistDispatcher` moves the
//! write to a dedicated worker (spawned through [`BlockingTaskSpawner`]) and
//! coalesces redundant snapshots so a burst of events only triggers a single
//! disk write per "latest" state.
//!
//! The worker writes session and workspace state with [`crate::persistence`]'s
//! atomic writers; if a write fails the error is reported through
//! `tracing::warn!` and the dispatcher continues — failure on disk must not
//! tear down the runtime.

use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, PoisonError},
    time::{Duration, Instant},
};

use gwt::{save_session_state, save_workspace_state};

use super::BlockingTaskSpawner;

/// Snapshot of state that the persist worker should flush to disk.
#[derive(Debug, Clone)]
pub(crate) struct PersistSnapshot {
    pub(crate) session_path: PathBuf,
    pub(crate) session: gwt::PersistedSessionState,
    pub(crate) workspaces: Vec<(PathBuf, gwt::PersistedWorkspaceState)>,
}

#[derive(Default)]
struct DispatcherState {
    latest: Option<PersistSnapshot>,
    shutdown: bool,
    enqueued: u64,
    completed: u64,
    last_error: Option<String>,
}

struct DispatcherInner {
    state: Mutex<DispatcherState>,
    cond: Condvar,
}

/// Owner handle: enqueue snapshots and (in tests) wait until the worker drains.
pub(crate) struct PersistDispatcher {
    inner: Arc<DispatcherInner>,
}

/// Bounded wait on shutdown so process exit cannot drop a pending snapshot on
/// the floor while still guaranteeing that a stuck disk (e.g. Defender holding
/// an open handle) does not hang the shutdown indefinitely. Mirrors the
/// pre-Phase-B contract where `persist()` was synchronous and durable at
/// return.
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

impl Drop for PersistDispatcher {
    fn drop(&mut self) {
        // Signal shutdown so the worker drains the latest snapshot (if any)
        // and exits its loop.
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.shutdown = true;
        }
        self.inner.cond.notify_all();

        // Wait for the worker to flush every enqueued snapshot before we
        // return. Without this, callers that relied on the synchronous
        // pre-Phase-B `persist()` (state was durable at return time) could
        // lose the most recent write when the process exits immediately
        // after a state change.
        let deadline = Instant::now() + SHUTDOWN_DRAIN_TIMEOUT;
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while state.enqueued > state.completed {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                tracing::warn!(
                    pending = state.enqueued - state.completed,
                    "persist dispatcher drop timed out before drain completed"
                );
                break;
            }
            let (next_state, timed_out) = self
                .inner
                .cond
                .wait_timeout(state, remaining)
                .unwrap_or_else(|err| {
                    let (guard, timed_out) = err.into_inner();
                    (guard, timed_out)
                });
            state = next_state;
            if timed_out.timed_out() {
                if state.enqueued > state.completed {
                    tracing::warn!(
                        pending = state.enqueued - state.completed,
                        "persist dispatcher drop timed out before drain completed"
                    );
                }
                break;
            }
        }
    }
}

impl PersistDispatcher {
    pub(crate) fn new(spawner: &BlockingTaskSpawner) -> Self {
        let inner = Arc::new(DispatcherInner {
            state: Mutex::new(DispatcherState::default()),
            cond: Condvar::new(),
        });
        let worker_inner = inner.clone();
        spawner.spawn(move || worker_loop(worker_inner));
        Self { inner }
    }

    /// Coalesce-enqueue a snapshot. The worker only writes the most recent
    /// snapshot it sees, so a burst of calls collapses to a single write.
    pub(crate) fn enqueue(&self, snapshot: PersistSnapshot) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.enqueued = state.enqueued.saturating_add(1);
        state.latest = Some(snapshot);
        drop(state);
        self.inner.cond.notify_one();
    }

    #[cfg(test)]
    pub(crate) fn wait_idle(&self, timeout: Duration) -> bool {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let deadline = Instant::now() + timeout;
        while state.enqueued > state.completed {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return state.enqueued == state.completed;
            }
            let (next_state, timed_out) = self
                .inner
                .cond
                .wait_timeout(state, remaining)
                .unwrap_or_else(|err| {
                    let (guard, timed_out) = err.into_inner();
                    (guard, timed_out)
                });
            state = next_state;
            if timed_out.timed_out() {
                return state.enqueued == state.completed;
            }
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn last_error(&self) -> Option<String> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last_error
            .clone()
    }
}

fn worker_loop(inner: Arc<DispatcherInner>) {
    loop {
        let (snapshot, covered) = {
            let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
            while state.latest.is_none() && !state.shutdown {
                state = inner
                    .cond
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if state.latest.is_none() && state.shutdown {
                return;
            }
            (state.latest.take(), state.enqueued)
        };

        let outcome = if let Some(snap) = snapshot {
            write_snapshot(&snap)
        } else {
            Ok(())
        };

        let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
        if covered > state.completed {
            state.completed = covered;
        }
        if let Err(error) = outcome {
            tracing::warn!(error = %error, "persist dispatcher failed to write snapshot");
            state.last_error = Some(error.to_string());
        }
        let should_exit = state.shutdown && state.latest.is_none();
        drop(state);
        inner.cond.notify_all();
        if should_exit {
            return;
        }
    }
}

fn write_snapshot(snap: &PersistSnapshot) -> std::io::Result<()> {
    save_session_state(&snap.session_path, &snap.session)?;
    for (path, ws) in &snap.workspaces {
        save_workspace_state(path, ws)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::tempdir;

    use super::*;
    use crate::app_runtime::BlockingTaskSpawner;
    use gwt::{empty_workspace_state, load_session_state};

    fn sample_empty_session() -> gwt::PersistedSessionState {
        gwt::PersistedSessionState {
            tabs: Vec::new(),
            active_tab_id: None,
            recent_projects: Vec::new(),
        }
    }

    #[test]
    fn enqueue_returns_immediately_so_callers_do_not_block_on_disk_write() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-state.json");
        let dispatcher = PersistDispatcher::new(&BlockingTaskSpawner::thread());

        let started = std::time::Instant::now();
        for index in 0..200 {
            dispatcher.enqueue(PersistSnapshot {
                session_path: path.clone(),
                session: gwt::PersistedSessionState {
                    tabs: Vec::new(),
                    active_tab_id: Some(format!("tab-{index}")),
                    recent_projects: Vec::new(),
                },
                workspaces: Vec::new(),
            });
        }
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_millis(20),
            "200 enqueue calls should not synchronously wait for disk; took {elapsed:?}",
        );

        assert!(dispatcher.wait_idle(Duration::from_secs(5)));
        let on_disk = load_session_state(&path).expect("load persisted session");
        assert!(
            on_disk
                .active_tab_id
                .as_deref()
                .is_some_and(|id| id.starts_with("tab-")),
            "disk should hold a snapshot from the burst (got {:?})",
            on_disk.active_tab_id
        );
    }

    #[test]
    fn coalesces_burst_so_only_latest_snapshot_persists() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-state.json");
        let dispatcher = PersistDispatcher::new(&BlockingTaskSpawner::thread());

        for index in 0..50 {
            dispatcher.enqueue(PersistSnapshot {
                session_path: path.clone(),
                session: gwt::PersistedSessionState {
                    tabs: Vec::new(),
                    active_tab_id: Some(format!("tab-{index}")),
                    recent_projects: Vec::new(),
                },
                workspaces: Vec::new(),
            });
        }
        assert!(dispatcher.wait_idle(Duration::from_secs(5)));

        let on_disk = load_session_state(&path).expect("load persisted session");
        assert_eq!(
            on_disk.active_tab_id.as_deref(),
            Some("tab-49"),
            "coalesce should keep only the most recently enqueued snapshot",
        );
        assert!(dispatcher.last_error().is_none());
    }

    #[test]
    fn drop_waits_for_pending_snapshot_to_drain() {
        // Regression for #2694 PR review (P1): dropping the dispatcher must
        // flush a pending snapshot to disk before returning, otherwise an app
        // shutdown immediately after a state change loses the latest write.
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-state.json");
        let dispatcher = PersistDispatcher::new(&BlockingTaskSpawner::thread());

        dispatcher.enqueue(PersistSnapshot {
            session_path: path.clone(),
            session: gwt::PersistedSessionState {
                tabs: Vec::new(),
                active_tab_id: Some("durable".to_string()),
                recent_projects: Vec::new(),
            },
            workspaces: Vec::new(),
        });

        // Do NOT call wait_idle here — rely on Drop alone to drain.
        drop(dispatcher);

        let on_disk = load_session_state(&path).expect("load persisted session");
        assert_eq!(
            on_disk.active_tab_id.as_deref(),
            Some("durable"),
            "Drop must flush the pending snapshot before returning",
        );
    }

    #[test]
    fn writes_workspace_state_alongside_session_state() {
        let temp = tempdir().expect("tempdir");
        let session_path = temp.path().join("session-state.json");
        let workspace_path = temp.path().join("workspace.json");
        let dispatcher = PersistDispatcher::new(&BlockingTaskSpawner::thread());

        dispatcher.enqueue(PersistSnapshot {
            session_path: session_path.clone(),
            session: sample_empty_session(),
            workspaces: vec![(workspace_path.clone(), empty_workspace_state())],
        });
        assert!(dispatcher.wait_idle(Duration::from_secs(5)));

        assert!(session_path.exists(), "session file should be written");
        assert!(workspace_path.exists(), "workspace file should be written");
    }
}

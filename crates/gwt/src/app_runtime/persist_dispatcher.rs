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
    collections::VecDeque,
    path::PathBuf,
    sync::{mpsc, Arc, Condvar, Mutex, PoisonError},
    time::{Duration, Instant},
};

use gwt::{save_session_state, save_workspace_state, save_workspace_state_durable};

use super::BlockingTaskSpawner;

/// Snapshot of state that the persist worker should flush to disk.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PersistSnapshot {
    pub(crate) session_path: PathBuf,
    pub(crate) session: gwt::PersistedSessionState,
    pub(crate) workspaces: Vec<(PathBuf, gwt::PersistedWindowCanvasState)>,
}

#[derive(Default)]
struct DispatcherState {
    latest: Option<PersistSnapshot>,
    /// Generation of `latest`; durable requests use the same sequence so the
    /// worker can preserve enqueue order without giving up snapshot coalescing.
    latest_generation: Option<u64>,
    /// Non-coalescible workspace barriers. Each caller waits on its completion
    /// channel, so these requests must remain FIFO and must never be replaced.
    durable_workspaces: VecDeque<DurableWorkspaceRequest>,
    /// `Some(timestamp)` when an unwritten `latest` snapshot exists. Used by
    /// the worker to enforce the 50ms coalesce window: new enqueues that
    /// arrive within the window push the deadline forward so a quick burst
    /// collapses to a single disk write.
    latest_updated_at: Option<Instant>,
    shutdown: bool,
    enqueued: u64,
    completed: u64,
    last_successful_snapshot: Option<PersistSnapshot>,
    last_error: Option<String>,
}

struct DurableWorkspaceRequest {
    generation: u64,
    path: PathBuf,
    workspace: gwt::PersistedWindowCanvasState,
    completion: mpsc::SyncSender<Result<(), String>>,
}

enum PersistWork {
    Snapshot {
        generation: u64,
        snapshot: PersistSnapshot,
    },
    DurableWorkspace(DurableWorkspaceRequest),
}

struct DispatcherInner {
    state: Mutex<DispatcherState>,
    cond: Condvar,
    #[cfg(test)]
    before_workspace_write: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
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

/// Coalesce window: the worker waits up to this long after the most recent
/// `enqueue()` before writing the latest snapshot, so a burst of UI events
/// (resize / focus / viewport pan / arrange / ...) collapses to a single disk
/// write per window. On shutdown the worker drains immediately and skips the
/// wait.
const COALESCE_WINDOW: Duration = Duration::from_millis(50);

const DURABLE_BARRIER_TIMEOUT: Duration = Duration::from_secs(5);

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
            #[cfg(test)]
            before_workspace_write: Mutex::new(None),
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
        if state.latest.as_ref() == Some(&snapshot)
            || (state.latest.is_none()
                && state.last_successful_snapshot.as_ref() == Some(&snapshot))
        {
            return;
        }
        state.enqueued = state.enqueued.saturating_add(1);
        state.latest_generation = Some(state.enqueued);
        state.latest = Some(snapshot);
        state.latest_updated_at = Some(Instant::now());
        drop(state);
        self.inner.cond.notify_one();
    }

    /// Serialize an exact workspace state after every older dispatcher
    /// generation and wait until its crash-durable atomic write succeeds.
    pub(crate) fn flush_workspace_durable(
        &self,
        path: PathBuf,
        workspace: gwt::PersistedWindowCanvasState,
    ) -> std::io::Result<()> {
        let (completion, completed) = mpsc::sync_channel(1);
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if state.shutdown {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "persist dispatcher is shutting down",
                ));
            }
            state.enqueued = state.enqueued.saturating_add(1);
            let generation = state.enqueued;
            state.durable_workspaces.push_back(DurableWorkspaceRequest {
                generation,
                path,
                workspace,
                completion,
            });
        }
        self.inner.cond.notify_one();

        match completed.recv_timeout(DURABLE_BARRIER_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(std::io::Error::other(error)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timed out waiting for durable workspace persistence",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "persist dispatcher stopped before durable workspace persistence completed",
            )),
        }
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

    #[cfg(test)]
    pub(crate) fn enqueued_count(&self) -> u64 {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .enqueued
    }

    #[cfg(test)]
    pub(crate) fn completed_count(&self) -> u64 {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .completed
    }

    #[cfg(test)]
    fn set_before_workspace_write_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self
            .inner
            .before_workspace_write
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(hook);
    }
}

fn worker_loop(inner: Arc<DispatcherInner>) {
    loop {
        let work = {
            let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                let durable_is_next = state.durable_workspaces.front().is_some_and(|durable| {
                    state
                        .latest_generation
                        .is_none_or(|latest| durable.generation <= latest)
                });
                if durable_is_next {
                    break;
                }
                if state.shutdown {
                    break;
                }
                match state.latest_updated_at {
                    None => {
                        // No pending snapshot; sleep until enqueue or
                        // shutdown wakes us.
                        state = inner
                            .cond
                            .wait(state)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                    Some(updated_at) => {
                        let remaining = COALESCE_WINDOW.saturating_sub(updated_at.elapsed());
                        if remaining.is_zero() {
                            // Coalesce window elapsed; flush the snapshot.
                            break;
                        }
                        // Wait the remaining window. A new enqueue notifies
                        // us, we re-check the timestamp on the next loop
                        // iteration so the window restarts from the latest
                        // enqueue.
                        let (next_state, _timed_out) = inner
                            .cond
                            .wait_timeout(state, remaining)
                            .unwrap_or_else(|err| err.into_inner());
                        state = next_state;
                    }
                }
            }
            if state.latest.is_none() && state.durable_workspaces.is_empty() && state.shutdown {
                return;
            }
            let durable_is_next = state.durable_workspaces.front().is_some_and(|durable| {
                state
                    .latest_generation
                    .is_none_or(|latest| durable.generation <= latest)
            });
            if durable_is_next {
                PersistWork::DurableWorkspace(
                    state
                        .durable_workspaces
                        .pop_front()
                        .expect("durable workspace request should exist"),
                )
            } else {
                state.latest_updated_at = None;
                PersistWork::Snapshot {
                    generation: state
                        .latest_generation
                        .take()
                        .expect("latest snapshot generation should exist"),
                    snapshot: state.latest.take().expect("latest snapshot should exist"),
                }
            }
        };

        let (generation, outcome, successful_snapshot, durable_completion) = match work {
            PersistWork::Snapshot {
                generation,
                snapshot,
            } => {
                let outcome = write_snapshot(&snapshot, &inner).map_err(|error| error.to_string());
                let successful_snapshot = outcome.is_ok().then_some(snapshot);
                (generation, outcome, successful_snapshot, None)
            }
            PersistWork::DurableWorkspace(request) => {
                let outcome = save_workspace_state_durable(&request.path, &request.workspace)
                    .map_err(|error| error.to_string());
                (request.generation, outcome, None, Some(request.completion))
            }
        };

        let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
        if generation > state.completed {
            state.completed = generation;
        }
        if let Some(snap) = successful_snapshot {
            state.last_successful_snapshot = Some(snap);
            state.last_error = None;
        } else if durable_completion.is_some() && outcome.is_ok() {
            state.last_successful_snapshot = None;
            state.last_error = None;
        } else if let Err(error) = &outcome {
            if durable_completion.is_some() {
                state.last_successful_snapshot = None;
            }
            tracing::warn!(error = %error, "persist dispatcher failed to write snapshot");
            state.last_error = Some(error.clone());
        }
        let should_exit =
            state.shutdown && state.latest.is_none() && state.durable_workspaces.is_empty();
        drop(state);
        inner.cond.notify_all();
        if let Some(completion) = durable_completion {
            let _ = completion.send(outcome);
        }
        if should_exit {
            return;
        }
    }
}

fn write_snapshot(snap: &PersistSnapshot, _inner: &DispatcherInner) -> std::io::Result<()> {
    save_session_state(&snap.session_path, &snap.session)?;
    for (path, ws) in &snap.workspaces {
        #[cfg(test)]
        if let Some(hook) = _inner
            .before_workspace_write
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            hook();
        }
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
    use gwt::{
        default_workspace_state, empty_workspace_state, load_session_state, load_workspace_state,
        WindowPreset,
    };

    fn sample_empty_session() -> gwt::PersistedSessionState {
        gwt::PersistedSessionState {
            tabs: Vec::new(),
            active_tab_id: None,
            recent_projects: Vec::new(),
        }
    }

    fn sample_session(active_tab_id: &str) -> gwt::PersistedSessionState {
        gwt::PersistedSessionState {
            tabs: Vec::new(),
            active_tab_id: Some(active_tab_id.to_string()),
            recent_projects: Vec::new(),
        }
    }

    fn sample_snapshot(session_path: PathBuf, active_tab_id: &str) -> PersistSnapshot {
        PersistSnapshot {
            session_path,
            session: sample_session(active_tab_id),
            workspaces: Vec::new(),
        }
    }

    fn unstarted_dispatcher() -> PersistDispatcher {
        PersistDispatcher {
            inner: Arc::new(DispatcherInner {
                state: Mutex::new(DispatcherState::default()),
                cond: Condvar::new(),
                before_workspace_write: Mutex::new(None),
            }),
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
    fn worker_waits_full_coalesce_window_before_writing() {
        // The first snapshot's deadline is COALESCE_WINDOW after enqueue;
        // a follow-up snapshot inside that window must postpone the write so
        // a quick burst (resize / focus / viewport pan / arrange) collapses
        // to a single disk hit instead of producing one write per call.
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-state.json");
        let dispatcher = PersistDispatcher::new(&BlockingTaskSpawner::thread());

        let started = std::time::Instant::now();
        dispatcher.enqueue(PersistSnapshot {
            session_path: path.clone(),
            session: gwt::PersistedSessionState {
                tabs: Vec::new(),
                active_tab_id: Some("first".to_string()),
                recent_projects: Vec::new(),
            },
            workspaces: Vec::new(),
        });
        std::thread::sleep(Duration::from_millis(20));
        dispatcher.enqueue(PersistSnapshot {
            session_path: path.clone(),
            session: gwt::PersistedSessionState {
                tabs: Vec::new(),
                active_tab_id: Some("second".to_string()),
                recent_projects: Vec::new(),
            },
            workspaces: Vec::new(),
        });

        assert!(dispatcher.wait_idle(Duration::from_secs(5)));
        let elapsed = started.elapsed();

        let on_disk = load_session_state(&path).expect("load persisted session");
        assert_eq!(
            on_disk.active_tab_id.as_deref(),
            Some("second"),
            "coalesce window should keep only the latest snapshot",
        );
        assert!(
            elapsed >= Duration::from_millis(50),
            "writer should respect the 50ms coalesce window from the most recent enqueue (elapsed = {elapsed:?})",
        );
    }

    #[test]
    fn suppresses_identical_snapshot_after_successful_write() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-state.json");
        let dispatcher = PersistDispatcher::new(&BlockingTaskSpawner::thread());
        let snapshot = sample_snapshot(path.clone(), "stable");

        dispatcher.enqueue(snapshot.clone());
        assert!(dispatcher.wait_idle(Duration::from_secs(5)));
        assert_eq!(dispatcher.completed_count(), 1);

        for _ in 0..25 {
            dispatcher.enqueue(snapshot.clone());
        }
        assert!(dispatcher.wait_idle(Duration::from_millis(100)));

        assert_eq!(
            dispatcher.enqueued_count(),
            1,
            "identical snapshots that already persisted should not enqueue new disk work",
        );
        assert_eq!(
            dispatcher.completed_count(),
            1,
            "identical snapshots that already persisted should not complete extra writes",
        );

        dispatcher.enqueue(sample_snapshot(path.clone(), "changed"));
        assert!(dispatcher.wait_idle(Duration::from_secs(5)));
        let on_disk = load_session_state(&path).expect("load persisted session");
        assert_eq!(
            on_disk.active_tab_id.as_deref(),
            Some("changed"),
            "changed snapshots must still persist after duplicate suppression",
        );
        assert_eq!(dispatcher.completed_count(), 2);
    }

    #[test]
    fn suppresses_identical_snapshot_while_pending() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-state.json");
        let dispatcher = unstarted_dispatcher();
        let snapshot = sample_snapshot(path.clone(), "pending");

        dispatcher.enqueue(snapshot.clone());
        let first_updated_at = dispatcher
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .latest_updated_at
            .expect("first enqueue should create pending timestamp");
        std::thread::sleep(Duration::from_millis(20));
        dispatcher.enqueue(snapshot);

        let mut state = dispatcher
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            state.enqueued, 1,
            "identical pending snapshots should not increment enqueue count",
        );
        assert_eq!(
            state.latest_updated_at,
            Some(first_updated_at),
            "identical pending enqueue should not restart the coalesce window",
        );
        assert_eq!(
            state.latest.as_ref(),
            Some(&sample_snapshot(path, "pending")),
            "original pending snapshot should remain queued",
        );
        state.completed = state.enqueued;
    }

    #[test]
    fn failed_write_does_not_suppress_later_retry() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-state.json");
        std::fs::create_dir(&path).expect("create blocking directory");
        let dispatcher = PersistDispatcher::new(&BlockingTaskSpawner::thread());
        let snapshot = sample_snapshot(path.clone(), "retryable");

        dispatcher.enqueue(snapshot.clone());
        assert!(dispatcher.wait_idle(Duration::from_secs(5)));
        assert_eq!(dispatcher.completed_count(), 1);
        assert!(
            dispatcher.last_error().is_some(),
            "first write should fail so the snapshot must not enter the successful duplicate cache",
        );

        std::fs::remove_dir(&path).expect("remove blocking directory");
        dispatcher.enqueue(snapshot);
        assert!(dispatcher.wait_idle(Duration::from_secs(5)));

        let on_disk = load_session_state(&path).expect("load persisted session");
        assert_eq!(on_disk.active_tab_id.as_deref(), Some("retryable"));
        assert_eq!(
            dispatcher.completed_count(),
            2,
            "identical snapshot after a failed write must retry instead of being suppressed",
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
    fn dispatcher_runs_on_tokio_spawn_blocking_pool() {
        // Production wires the dispatcher through `BlockingTaskSpawner::tokio`
        // (`spawn_blocking` on the tao/winit tokio runtime). The thread-based
        // variant covers the algorithmic contract, but the production
        // execution path needs its own coverage so a future regression in
        // spawn_blocking semantics (cancellation, executor shutdown order)
        // surfaces here.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-state.json");

        {
            let dispatcher =
                PersistDispatcher::new(&BlockingTaskSpawner::tokio(runtime.handle().clone()));
            for index in 0..25 {
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
                Some("tab-24"),
                "tokio spawner must coalesce identically to the thread spawner",
            );
            assert!(dispatcher.last_error().is_none());
            // Drop the dispatcher while the tokio runtime is still alive so
            // the Drop drain has a worker to wait on.
        }

        // Shutting the tokio runtime down here proves the worker
        // (spawn_blocking task) cooperated with shutdown after Drop signalled.
        runtime.shutdown_timeout(Duration::from_secs(2));
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

    #[test]
    fn durable_workspace_barrier_orders_exact_window_after_inflight_snapshot() {
        let temp = tempdir().expect("tempdir");
        let session_path = temp.path().join("session-state.json");
        let workspace_path = temp.path().join("workspace.json");
        let dispatcher = Arc::new(PersistDispatcher::new(&BlockingTaskSpawner::thread()));

        let (old_write_ready_tx, old_write_ready_rx) = std::sync::mpsc::sync_channel(1);
        let (release_old_write_tx, release_old_write_rx) = std::sync::mpsc::sync_channel(1);
        let release_old_write_rx = Arc::new(Mutex::new(release_old_write_rx));
        dispatcher.set_before_workspace_write_hook(Arc::new(move || {
            old_write_ready_tx
                .send(())
                .expect("report old workspace write ready");
            release_old_write_rx
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv()
                .expect("release old workspace write");
        }));

        dispatcher.enqueue(PersistSnapshot {
            session_path,
            session: sample_empty_session(),
            workspaces: vec![(workspace_path.clone(), empty_workspace_state())],
        });
        old_write_ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("old generation should pause before workspace write");

        let mut exact_workspace = default_workspace_state();
        exact_workspace.windows[0].id = "agent-delivery-exact".to_string();
        exact_workspace.windows[0].preset = WindowPreset::Agent;
        let barrier_dispatcher = dispatcher.clone();
        let barrier_workspace_path = workspace_path.clone();
        let barrier_workspace = exact_workspace.clone();
        let (barrier_done_tx, barrier_done_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = barrier_dispatcher
                .flush_workspace_durable(barrier_workspace_path, barrier_workspace);
            barrier_done_tx
                .send(result)
                .expect("report durable barrier result");
        });

        let early_result = barrier_done_rx.recv_timeout(Duration::from_millis(100));
        let returned_before_old_write = early_result.is_ok();
        release_old_write_tx
            .send(())
            .expect("allow old workspace write");
        let barrier_result = match early_result {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => barrier_done_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("durable barrier should finish after old generation"),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("durable barrier worker disconnected")
            }
        };
        barrier_result.expect("durable barrier");
        assert!(dispatcher.wait_idle(Duration::from_secs(5)));

        let on_disk = load_workspace_state(&workspace_path).expect("load durable workspace");
        let exact_window_is_recoverable = on_disk
            .windows
            .iter()
            .any(|window| window.id == "agent-delivery-exact");
        assert!(
            !returned_before_old_write && exact_window_is_recoverable,
            "barrier returned before the old generation was ordered ({returned_before_old_write}); \
             exact launch window remained recoverable ({exact_window_is_recoverable})",
        );
    }
}

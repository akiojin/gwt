use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use crate::{app_runtime::AppEventProxy, UserEvent};

pub use gwt::IndexRebuildScope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectIndexBootstrapRequest {
    Spawned,
    AlreadyRunning,
    SpawnFailed,
    /// SPEC-2359 W-17 (FR-400): a bootstrap for this project completed within
    /// the cooldown window — the cached status was replayed instead of
    /// re-running the bootstrap + status sweep (reconnect-storm guard).
    SkippedFresh,
}

const FULL_STATUS_RETRY_DELAY: Duration = Duration::from_millis(100);
const FULL_STATUS_COOLDOWN: Duration = Duration::from_secs(10);
/// SPEC-2359 W-17 (FR-400): how long a completed startup bootstrap satisfies
/// repeat `frontend_ready` requests (reconnect storms replay it on every
/// re-established socket) before a real re-run is allowed again.
const BOOTSTRAP_STATUS_COOLDOWN: Duration = Duration::from_secs(120);

type BootstrapFn = dyn Fn(&Path) -> Result<(), String> + Send + Sync + 'static;
type StatusProbeFn = dyn Fn(&Path) -> gwt::ProjectIndexStatusView + Send + Sync + 'static;

/// Identifies a unit of background work tracked by
/// [`ProjectIndexBootstrapService::in_flight`].
///
/// `Bootstrap` covers startup bootstrap + current-worktree status probe.
/// `FullStatus` covers Settings.Index full-table refreshes; duplicate full
/// refresh requests collapse into the in-flight refresh instead of queuing a
/// second all-worktree probe. `FullStatusRetry` covers a queued Settings.Index
/// full-table refresh that should run after an already in-flight bootstrap
/// releases. `Rebuild` covers per-cell rebuilds keyed by `(project_root, scope, worktree_hash?)`
/// so different
/// scopes/worktrees can run in parallel while same-key duplicates are
/// coalesced.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IndexInFlightKey {
    Bootstrap {
        project_root: PathBuf,
    },
    FullStatus {
        project_root: PathBuf,
    },
    FullStatusRetry {
        project_root: PathBuf,
    },
    Rebuild {
        project_root: PathBuf,
        scope: IndexRebuildScope,
        worktree_hash: Option<String>,
    },
}

#[derive(Clone)]
pub struct ProjectIndexBootstrapService {
    in_flight: Arc<Mutex<HashSet<IndexInFlightKey>>>,
    last_full_status: Arc<Mutex<HashMap<PathBuf, FullStatusCacheEntry>>>,
    full_status_cooldown: Duration,
    last_bootstrap_status: Arc<Mutex<HashMap<PathBuf, FullStatusCacheEntry>>>,
    bootstrap_status_cooldown: Duration,
}

#[derive(Clone)]
struct FullStatusCacheEntry {
    refreshed_at: Instant,
    status: gwt::ProjectIndexStatusView,
}

impl Default for ProjectIndexBootstrapService {
    fn default() -> Self {
        Self {
            in_flight: Arc::default(),
            last_full_status: Arc::default(),
            full_status_cooldown: FULL_STATUS_COOLDOWN,
            last_bootstrap_status: Arc::default(),
            bootstrap_status_cooldown: BOOTSTRAP_STATUS_COOLDOWN,
        }
    }
}

impl ProjectIndexBootstrapService {
    pub(crate) fn global() -> &'static Self {
        static SERVICE: OnceLock<ProjectIndexBootstrapService> = OnceLock::new();
        SERVICE.get_or_init(Self::default)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        // Legacy tests re-spawn bootstraps freely; the reconnect-storm
        // cooldown is exercised explicitly via
        // `new_for_test_with_bootstrap_cooldown`.
        Self {
            bootstrap_status_cooldown: Duration::ZERO,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test_with_bootstrap_cooldown(
        bootstrap_status_cooldown: Duration,
    ) -> Self {
        Self {
            bootstrap_status_cooldown,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test_with_full_status_cooldown(full_status_cooldown: Duration) -> Self {
        Self {
            full_status_cooldown,
            bootstrap_status_cooldown: Duration::ZERO,
            ..Self::default()
        }
    }

    pub(crate) fn spawn(
        &self,
        proxy: AppEventProxy,
        project_root: PathBuf,
    ) -> ProjectIndexBootstrapRequest {
        self.spawn_with(
            proxy,
            project_root,
            gwt::index_worker::bootstrap_project_index_for_path,
            current_worktree_status_probe,
        )
    }

    pub(crate) fn spawn_full_status_refresh(
        &self,
        proxy: AppEventProxy,
        project_root: PathBuf,
    ) -> ProjectIndexBootstrapRequest {
        self.spawn_full_status_refresh_with_retry(
            proxy,
            project_root,
            Arc::new(gwt::index_worker::bootstrap_project_index_for_path),
            Arc::new(cached_aggregate_status_probe),
            FULL_STATUS_RETRY_DELAY,
        )
    }

    fn spawn_full_status_refresh_with_retry(
        &self,
        proxy: AppEventProxy,
        project_root: PathBuf,
        bootstrap: Arc<BootstrapFn>,
        status_probe: Arc<StatusProbeFn>,
        retry_delay: Duration,
    ) -> ProjectIndexBootstrapRequest {
        let project_key = normalize_project_root(&project_root);
        let bootstrap_key = IndexInFlightKey::Bootstrap {
            project_root: project_key.clone(),
        };
        let bootstrap_was_running = self.is_reserved(&bootstrap_key);
        let full_status_key = IndexInFlightKey::FullStatus {
            project_root: project_key,
        };
        let request = self.spawn_full_status_refresh_once_with(
            proxy.clone(),
            project_root.clone(),
            bootstrap.clone(),
            status_probe.clone(),
        );
        if request != ProjectIndexBootstrapRequest::AlreadyRunning {
            return request;
        }
        let bootstrap_is_running = self.is_reserved(&bootstrap_key);
        let full_status_is_running = self.is_reserved(&full_status_key);
        if !bootstrap_was_running && (!bootstrap_is_running || full_status_is_running) {
            return ProjectIndexBootstrapRequest::AlreadyRunning;
        }
        self.queue_full_status_refresh_retry(
            proxy,
            project_root,
            bootstrap,
            status_probe,
            retry_delay,
        )
    }

    fn spawn_full_status_refresh_once_with(
        &self,
        proxy: AppEventProxy,
        project_root: PathBuf,
        bootstrap: Arc<BootstrapFn>,
        status_probe: Arc<StatusProbeFn>,
    ) -> ProjectIndexBootstrapRequest {
        let project_key = normalize_project_root(&project_root);
        let project_root_label = project_key.display().to_string();
        let bootstrap_key = IndexInFlightKey::Bootstrap {
            project_root: project_key.clone(),
        };
        if self.is_reserved(&bootstrap_key) {
            tracing::debug!(
                target: "gwt::index",
                worktree = %project_root_label,
                "project index full status refresh waiting for startup bootstrap"
            );
            return ProjectIndexBootstrapRequest::AlreadyRunning;
        }
        if let Some(status) = self.fresh_full_status(&project_key, &project_root_label) {
            proxy.send(UserEvent::ProjectIndexStatus {
                project_root: project_root_label,
                status,
            });
            return ProjectIndexBootstrapRequest::AlreadyRunning;
        }
        let key = IndexInFlightKey::FullStatus {
            project_root: project_key.clone(),
        };
        if !self.try_reserve(key.clone()) {
            tracing::debug!(
                target: "gwt::index",
                worktree = %project_root_label,
                "project index full status refresh already running"
            );
            return ProjectIndexBootstrapRequest::AlreadyRunning;
        }

        let in_flight = self.in_flight.clone();
        let key_for_thread = key.clone();
        let service_for_thread = self.clone();
        let spawn_result = thread::Builder::new()
            .name("gwt-index-full-status-refresh".to_string())
            .spawn(move || {
                let _guard = InFlightGuard {
                    in_flight,
                    key: key_for_thread,
                };
                let bootstrap_started = Instant::now();
                match bootstrap(&project_key) {
                    Ok(()) => {
                        let bootstrap_elapsed_ms = bootstrap_started.elapsed().as_millis() as u64;
                        tracing::info!(
                            target: "gwt::index",
                            worktree = %project_root_label,
                            elapsed_ms = bootstrap_elapsed_ms,
                            "project index full status refresh bootstrap completed"
                        );

                        let status_started = Instant::now();
                        let status = status_probe(&project_key);
                        tracing::info!(
                            target: "gwt::index",
                            worktree = %project_root_label,
                            elapsed_ms = status_started.elapsed().as_millis() as u64,
                            state = %status.state,
                            "project index full status refreshed"
                        );
                        let kick_orchestrator =
                            status.state == gwt::ProjectIndexStatusState::RepairRequired;
                        service_for_thread.emit_full_status_if_changed(
                            &proxy,
                            &project_key,
                            &project_root_label,
                            status.clone(),
                        );
                        if kick_orchestrator {
                            trigger_auto_repair_for_project(
                                service_for_thread,
                                proxy.clone(),
                                project_key.clone(),
                                &status,
                            );
                        }
                    }
                    Err(error) => {
                        let elapsed_ms = bootstrap_started.elapsed().as_millis() as u64;
                        tracing::warn!(
                            target: "gwt::index",
                            worktree = %project_root_label,
                            elapsed_ms,
                            error = %error,
                            "project index full status refresh bootstrap failed"
                        );
                        let status = gwt::ProjectIndexStatusView::new(
                            gwt::ProjectIndexStatusState::Error,
                            format!(
                                "Project index full status refresh failed after {elapsed_ms} ms: {error}"
                            ),
                        );
                        service_for_thread.emit_full_status_if_changed(
                            &proxy,
                            &project_key,
                            &project_root_label,
                            status,
                        );
                    }
                }
            });

        match spawn_result {
            Ok(_) => ProjectIndexBootstrapRequest::Spawned,
            Err(error) => {
                self.release(&key);
                tracing::warn!(
                    target: "gwt::index",
                    error = %error,
                    "failed to spawn project index full status refresh background task"
                );
                ProjectIndexBootstrapRequest::SpawnFailed
            }
        }
    }

    fn queue_full_status_refresh_retry(
        &self,
        proxy: AppEventProxy,
        project_root: PathBuf,
        bootstrap: Arc<BootstrapFn>,
        status_probe: Arc<StatusProbeFn>,
        retry_delay: Duration,
    ) -> ProjectIndexBootstrapRequest {
        let project_key = normalize_project_root(&project_root);
        let project_root_label = project_key.display().to_string();
        let retry_key = IndexInFlightKey::FullStatusRetry {
            project_root: project_key.clone(),
        };
        if !self.try_reserve(retry_key.clone()) {
            tracing::debug!(
                target: "gwt::index",
                worktree = %project_root_label,
                "project index full status refresh retry already queued"
            );
            return ProjectIndexBootstrapRequest::AlreadyRunning;
        }

        let in_flight = self.in_flight.clone();
        let retry_key_for_thread = retry_key.clone();
        let service_for_thread = self.clone();
        let spawn_result = thread::Builder::new()
            .name("gwt-index-full-status-retry".to_string())
            .spawn(move || {
                let _guard = InFlightGuard {
                    in_flight,
                    key: retry_key_for_thread,
                };
                let started = Instant::now();
                loop {
                    thread::sleep(retry_delay);
                    match service_for_thread.spawn_full_status_refresh_once_with(
                        proxy.clone(),
                        project_key.clone(),
                        bootstrap.clone(),
                        status_probe.clone(),
                    ) {
                        ProjectIndexBootstrapRequest::Spawned => {
                            tracing::info!(
                                target: "gwt::index",
                                worktree = %project_root_label,
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                "queued project index full status refresh spawned after bootstrap"
                            );
                            return;
                        }
                        ProjectIndexBootstrapRequest::SpawnFailed
                        | ProjectIndexBootstrapRequest::SkippedFresh => return,
                        ProjectIndexBootstrapRequest::AlreadyRunning => {}
                    }
                }
            });

        match spawn_result {
            Ok(_) => ProjectIndexBootstrapRequest::AlreadyRunning,
            Err(error) => {
                self.release(&retry_key);
                tracing::warn!(
                    target: "gwt::index",
                    error = %error,
                    "failed to queue project index full status refresh retry"
                );
                ProjectIndexBootstrapRequest::SpawnFailed
            }
        }
    }

    fn emit_full_status_if_changed(
        &self,
        proxy: &AppEventProxy,
        project_key: &Path,
        project_root_label: &str,
        status: gwt::ProjectIndexStatusView,
    ) -> bool {
        let key = normalize_project_root(project_key);
        let mut changed = true;
        if let Ok(mut last) = self.last_full_status.lock() {
            if last.get(&key).map(|entry| &entry.status) == Some(&status) {
                tracing::debug!(
                    target: "gwt::index",
                    worktree = %project_root_label,
                    state = %status.state,
                    "skipping unchanged project index full status broadcast"
                );
                changed = false;
            }
            last.insert(
                key,
                FullStatusCacheEntry {
                    refreshed_at: Instant::now(),
                    status: status.clone(),
                },
            );
        }
        if !changed {
            return false;
        }
        proxy.send(UserEvent::ProjectIndexStatus {
            project_root: project_root_label.to_string(),
            status,
        });
        true
    }

    fn fresh_full_status(
        &self,
        project_key: &Path,
        project_root_label: &str,
    ) -> Option<gwt::ProjectIndexStatusView> {
        if self.full_status_cooldown.is_zero() {
            return None;
        }
        let key = normalize_project_root(project_key);
        let Ok(last) = self.last_full_status.lock() else {
            return None;
        };
        let entry = last.get(&key)?;
        let elapsed = entry.refreshed_at.elapsed();
        if elapsed >= self.full_status_cooldown {
            return None;
        }
        tracing::debug!(
            target: "gwt::index",
            worktree = %project_root_label,
            elapsed_ms = elapsed.as_millis() as u64,
            cooldown_ms = self.full_status_cooldown.as_millis() as u64,
            "replaying fresh project index full status"
        );
        Some(entry.status.clone())
    }

    fn invalidate_full_status(&self, project_root: &Path) {
        let key = normalize_project_root(project_root);
        if let Ok(mut last) = self.last_full_status.lock() {
            last.remove(&key);
        }
        // Rebuilds change index health — the next frontend_ready must probe
        // for real instead of replaying a pre-rebuild bootstrap status.
        if let Ok(mut last) = self.last_bootstrap_status.lock() {
            last.remove(&key);
        }
    }

    /// SPEC-2359 W-17 (FR-400): cached status of a bootstrap that completed
    /// within `bootstrap_status_cooldown`, or `None` when a real run is due.
    fn bootstrap_completed_recently(
        &self,
        project_key: &Path,
        project_root_label: &str,
    ) -> Option<gwt::ProjectIndexStatusView> {
        if self.bootstrap_status_cooldown.is_zero() {
            return None;
        }
        let key = normalize_project_root(project_key);
        let last = self.last_bootstrap_status.lock().ok()?;
        let entry = last.get(&key)?;
        let elapsed = entry.refreshed_at.elapsed();
        if elapsed >= self.bootstrap_status_cooldown {
            return None;
        }
        tracing::debug!(
            target: "gwt::index",
            worktree = %project_root_label,
            elapsed_ms = elapsed.as_millis() as u64,
            cooldown_ms = self.bootstrap_status_cooldown.as_millis() as u64,
            "skipping fresh project index bootstrap; replaying cached status"
        );
        Some(entry.status.clone())
    }

    fn record_bootstrap_status(&self, project_root: &Path, status: &gwt::ProjectIndexStatusView) {
        let key = normalize_project_root(project_root);
        if let Ok(mut last) = self.last_bootstrap_status.lock() {
            last.insert(
                key,
                FullStatusCacheEntry {
                    refreshed_at: Instant::now(),
                    status: status.clone(),
                },
            );
        }
    }

    #[cfg(test)]
    fn invalidate_full_status_for_test(&self, project_root: &Path) {
        self.invalidate_full_status(project_root);
    }

    #[cfg(test)]
    fn full_status_is_idle_for_test(&self, project_root: &Path) -> bool {
        let project_key = normalize_project_root(project_root);
        let full_status_key = IndexInFlightKey::FullStatus {
            project_root: project_key.clone(),
        };
        let retry_key = IndexInFlightKey::FullStatusRetry {
            project_root: project_key,
        };
        !self.is_reserved(&full_status_key) && !self.is_reserved(&retry_key)
    }

    pub(crate) fn spawn_with<B, S>(
        &self,
        proxy: AppEventProxy,
        project_root: PathBuf,
        bootstrap: B,
        status_probe: S,
    ) -> ProjectIndexBootstrapRequest
    where
        B: FnOnce(&Path) -> Result<(), String> + Send + 'static,
        S: FnOnce(&Path) -> gwt::ProjectIndexStatusView + Send + 'static,
    {
        let project_key = normalize_project_root(&project_root);
        let project_root_label = project_key.display().to_string();
        // SPEC-2359 W-17 (FR-400): reconnect storms replay frontend_ready on
        // every re-established socket. A bootstrap that completed within the
        // cooldown satisfies the request from cache instead of re-running the
        // sweep — the cached status is replayed so the new page still
        // populates its status cell.
        if let Some(status) = self.bootstrap_completed_recently(&project_key, &project_root_label) {
            proxy.send(UserEvent::ProjectIndexStatus {
                project_root: project_root_label,
                status,
            });
            return ProjectIndexBootstrapRequest::SkippedFresh;
        }
        let key = IndexInFlightKey::Bootstrap {
            project_root: project_key.clone(),
        };
        if !self.try_reserve(key.clone()) {
            tracing::debug!(
                target: "gwt::index",
                worktree = %project_root_label,
                "project index bootstrap already running for worktree"
            );
            return ProjectIndexBootstrapRequest::AlreadyRunning;
        }

        let in_flight = self.in_flight.clone();
        let key_for_thread = key.clone();
        let service_for_thread = self.clone();
        let spawn_result = thread::Builder::new()
            .name("gwt-index-bootstrap".to_string())
            .spawn(move || {
                let _guard = InFlightGuard {
                    in_flight,
                    key: key_for_thread,
                };
                let bootstrap_started = Instant::now();
                match bootstrap(&project_key) {
                    Ok(()) => {
                        let bootstrap_elapsed_ms = bootstrap_started.elapsed().as_millis() as u64;
                        tracing::info!(
                            target: "gwt::index",
                            worktree = %project_root_label,
                            elapsed_ms = bootstrap_elapsed_ms,
                            "project index bootstrap completed in background"
                        );

                        let status_started = Instant::now();
                        let status = status_probe(&project_key);
                        tracing::info!(
                            target: "gwt::index",
                            worktree = %project_root_label,
                            elapsed_ms = status_started.elapsed().as_millis() as u64,
                            state = %status.state,
                            "project index status refreshed after background bootstrap"
                        );
                        let kick_orchestrator =
                            status.state == gwt::ProjectIndexStatusState::RepairRequired;
                        service_for_thread.record_bootstrap_status(&project_key, &status);
                        proxy.send(UserEvent::ProjectIndexStatus {
                            project_root: project_root_label.clone(),
                            status: status.clone(),
                        });
                        if kick_orchestrator {
                            trigger_auto_repair_for_project(
                                service_for_thread,
                                proxy.clone(),
                                project_key.clone(),
                                &status,
                            );
                        }
                    }
                    Err(error) => {
                        let elapsed_ms = bootstrap_started.elapsed().as_millis() as u64;
                        tracing::warn!(
                            target: "gwt::index",
                            worktree = %project_root_label,
                            elapsed_ms,
                            error = %error,
                            "project index bootstrap failed in background"
                        );
                        proxy.send(UserEvent::ProjectIndexStatus {
                            project_root: project_root_label,
                            status: gwt::ProjectIndexStatusView::new(
                                gwt::ProjectIndexStatusState::Error,
                                format!(
                                    "Project index bootstrap failed after {elapsed_ms} ms: {error}"
                                ),
                            ),
                        });
                    }
                }
            });

        match spawn_result {
            Ok(_) => ProjectIndexBootstrapRequest::Spawned,
            Err(error) => {
                self.release(&key);
                tracing::warn!(
                    target: "gwt::index",
                    error = %error,
                    "failed to spawn project index bootstrap background task"
                );
                ProjectIndexBootstrapRequest::SpawnFailed
            }
        }
    }

    /// Spawn a background task that performs a single per-cell rebuild for
    /// `(project_root, scope, worktree_hash?)`. The closure runs on the spawned
    /// thread and is responsible for emitting any per-cell `ProjectIndexStatus`
    /// events the caller needs (the service itself only handles deduplication).
    ///
    /// Returns `AlreadyRunning` if another rebuild for the same key is already
    /// in flight; bootstrap and rebuild tasks for the same project but
    /// different keys proceed in parallel.
    pub(crate) fn spawn_rebuild_with<R>(
        &self,
        project_root: PathBuf,
        scope: IndexRebuildScope,
        worktree_hash: Option<String>,
        rebuild: R,
    ) -> ProjectIndexBootstrapRequest
    where
        R: FnOnce() + Send + 'static,
    {
        let project_key = normalize_project_root(&project_root);
        let project_root_label = project_key.display().to_string();
        let key = IndexInFlightKey::Rebuild {
            project_root: project_key.clone(),
            scope,
            worktree_hash: worktree_hash.clone(),
        };
        if !self.try_reserve(key.clone()) {
            tracing::debug!(
                target: "gwt::index",
                worktree = %project_root_label,
                scope = scope.label(),
                worktree_hash = ?worktree_hash,
                "project index rebuild already running for cell"
            );
            return ProjectIndexBootstrapRequest::AlreadyRunning;
        }
        self.invalidate_full_status(&project_key);

        let in_flight = self.in_flight.clone();
        let key_for_thread = key.clone();
        let service_for_thread = self.clone();
        let project_key_for_thread = project_key.clone();
        let spawn_result = thread::Builder::new()
            .name("gwt-index-rebuild".to_string())
            .spawn(move || {
                let _guard = InFlightGuard {
                    in_flight,
                    key: key_for_thread,
                };
                rebuild();
                service_for_thread.invalidate_full_status(&project_key_for_thread);
            });

        match spawn_result {
            Ok(_) => ProjectIndexBootstrapRequest::Spawned,
            Err(error) => {
                self.release(&key);
                tracing::warn!(
                    target: "gwt::index",
                    error = %error,
                    "failed to spawn project index rebuild background task"
                );
                ProjectIndexBootstrapRequest::SpawnFailed
            }
        }
    }

    /// Synchronously acquire the rebuild key for `(project_root, scope,
    /// worktree_hash?)`, run `body`, then release. Returns `body`'s result,
    /// or an error string if the key is already held by another task. Used
    /// by [`ServiceBackedRebuildSpawner`] so orchestrator + per-cell IPC
    /// share the same dedup primitive.
    pub(crate) fn run_rebuild_with_lock<F>(
        &self,
        project_root: &Path,
        scope: IndexRebuildScope,
        worktree_hash: Option<&str>,
        body: F,
    ) -> Result<(), String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let key = IndexInFlightKey::Rebuild {
            project_root: normalize_project_root(project_root),
            scope,
            worktree_hash: worktree_hash.map(String::from),
        };
        if !self.try_reserve(key.clone()) {
            return Err(format!(
                "rebuild for scope={} worktree_hash={:?} is already in progress",
                scope.label(),
                worktree_hash
            ));
        }
        let result = body();
        self.invalidate_full_status(project_root);
        self.release(&key);
        result
    }

    fn try_reserve(&self, key: IndexInFlightKey) -> bool {
        let mut in_flight = self.in_flight.lock().expect("project index in-flight set");
        in_flight.insert(key)
    }

    fn is_reserved(&self, key: &IndexInFlightKey) -> bool {
        self.in_flight
            .lock()
            .expect("project index in-flight set")
            .contains(key)
    }

    fn release(&self, key: &IndexInFlightKey) {
        if let Ok(mut in_flight) = self.in_flight.lock() {
            in_flight.remove(key);
        }
    }
}

/// Wraps [`ProjectIndexBootstrapService`] and a rebuild runner so the
/// orchestrator and per-cell IPC share the same dedup + invocation path.
pub(crate) struct ServiceBackedRebuildSpawner {
    service: ProjectIndexBootstrapService,
    rebuild_runner: Arc<gwt::IndexRebuildRunnerFn>,
}

impl ServiceBackedRebuildSpawner {
    pub(crate) fn new(
        service: ProjectIndexBootstrapService,
        runner: Arc<gwt::IndexRebuildRunnerFn>,
    ) -> Self {
        Self {
            service,
            rebuild_runner: runner,
        }
    }

    pub(crate) fn with_default_runner(service: ProjectIndexBootstrapService) -> Self {
        Self::new(service, Arc::new(gwt::default_rebuild_runner))
    }
}

impl gwt::IndexRebuildSpawner for ServiceBackedRebuildSpawner {
    fn rebuild(
        &self,
        project_root: &Path,
        scope: IndexRebuildScope,
        worktree_hash: Option<&str>,
    ) -> Result<(), String> {
        self.service
            .run_rebuild_with_lock(project_root, scope, worktree_hash, || {
                (self.rebuild_runner)(project_root, scope, worktree_hash)
            })
    }
}

/// Spawn a per-cell rebuild for `(project_root, scope, worktree_hash?)` in
/// the background. The cell is keyed in the in-flight set so concurrent
/// requests for the same cell are coalesced; bootstrap and other cells
/// proceed in parallel. SPEC-1939 US-5 / T-IDX-102.
pub(crate) fn spawn_per_cell_rebuild(
    service: ProjectIndexBootstrapService,
    proxy: AppEventProxy,
    project_root: PathBuf,
    scope: IndexRebuildScope,
    worktree_hash: Option<String>,
) -> ProjectIndexBootstrapRequest {
    spawn_per_cell_rebuild_with(
        service,
        proxy,
        project_root,
        scope,
        worktree_hash,
        Arc::new(gwt::default_rebuild_runner),
        Arc::new(|path: &Path| -> gwt::ProjectIndexStatusView {
            gwt::global_aggregated_status_cache().invalidate(path);
            gwt::aggregate_project_index_status_for_path(path)
        }),
    )
}

/// Test-friendly variant of [`spawn_per_cell_rebuild`] that injects a custom
/// rebuild runner and final-status provider so unit tests can drive the IPC
/// path without invoking real Python.
pub(crate) fn spawn_per_cell_rebuild_with(
    service: ProjectIndexBootstrapService,
    proxy: AppEventProxy,
    project_root: PathBuf,
    scope: IndexRebuildScope,
    worktree_hash: Option<String>,
    rebuild_runner: Arc<gwt::IndexRebuildRunnerFn>,
    final_status_provider: Arc<
        dyn Fn(&Path) -> gwt::ProjectIndexStatusView + Send + Sync + 'static,
    >,
) -> ProjectIndexBootstrapRequest {
    // Canonicalise so the proxy events share the same project_root key the
    // bootstrap path uses (`spawn_with` -> `normalize_project_root`).
    // Without this, the frontend `indexStatusByProjectRoot` would keep two
    // separate entries for the same project (raw vs canonical path),
    // breaking Settings.Index real-time updates after a per-cell rebuild.
    let canonical_project_root = normalize_project_root(&project_root);
    let project_root_label = canonical_project_root.display().to_string();
    let project_root_for_closure = canonical_project_root.clone();
    let worktree_hash_for_closure = worktree_hash.clone();
    let proxy_for_closure = proxy.clone();
    let rebuild_runner_for_closure = rebuild_runner.clone();
    let final_status_for_closure = final_status_provider.clone();

    service.spawn_rebuild_with(canonical_project_root, scope, worktree_hash, move || {
        let started_at = chrono::Utc::now();
        // Optimistic transition: switch the badge to `repairing(0/1)`
        // immediately so observers see auto-rebuild start before the
        // rebuild actually completes.
        proxy_for_closure.send(UserEvent::ProjectIndexStatus {
            project_root: project_root_label.clone(),
            status: gwt::ProjectIndexStatusView {
                state: gwt::ProjectIndexStatusState::Repairing,
                detail: format!(
                    "Rebuilding {} (worktree={:?})",
                    scope.label(),
                    worktree_hash_for_closure
                ),
                repair_started_at: Some(started_at),
                progress: Some(gwt::RebuildProgress {
                    scopes_done: 0,
                    scopes_total: 1,
                }),
                scopes: gwt::ProjectIndexScopes::default(),
                worktrees: std::collections::BTreeMap::new(),
                coverage: None,
            },
        });

        let result = rebuild_runner_for_closure(
            &project_root_for_closure,
            scope,
            worktree_hash_for_closure.as_deref(),
        );

        let final_view = match &result {
            Ok(()) => final_status_for_closure(&project_root_for_closure),
            Err(error) => gwt::ProjectIndexStatusView::new(
                gwt::ProjectIndexStatusState::Error,
                format!("Rebuild {} failed: {error}", scope.label()),
            ),
        };
        proxy_for_closure.send(UserEvent::ProjectIndexStatus {
            project_root: project_root_label.clone(),
            status: final_view,
        });
    })
}

/// Trigger the auto-rebuild orchestrator for `project_root` if the freshly
/// emitted status reports `RepairRequired`. The orchestrator runs in its own
/// background thread and re-emits `ProjectIndexStatus` events through the
/// proxy; the global aggregator cache is invalidated before re-aggregating
/// the final status so observers see the post-rebuild health.
pub(crate) fn trigger_auto_repair_for_project(
    service: ProjectIndexBootstrapService,
    proxy: AppEventProxy,
    project_root: PathBuf,
    initial_status: &gwt::ProjectIndexStatusView,
) -> Option<thread::JoinHandle<()>> {
    if initial_status.state != gwt::ProjectIndexStatusState::RepairRequired {
        return None;
    }
    let project_root_label = project_root.display().to_string();
    let project_root_for_sink = project_root.clone();
    let event_sink = move |view: gwt::ProjectIndexStatusView| {
        proxy.send(UserEvent::ProjectIndexStatus {
            project_root: project_root_for_sink.display().to_string(),
            status: view,
        });
    };
    let final_status_provider = |path: &Path| -> gwt::ProjectIndexStatusView {
        gwt::global_aggregated_status_cache().invalidate(path);
        gwt::aggregate_current_worktree_index_status_for_path(path)
    };
    let targets = gwt::collect_unhealthy_rebuild_targets_for_project_root(
        &initial_status.scopes,
        &project_root,
    );
    if targets.is_empty() {
        tracing::info!(
            target: "gwt::index",
            worktree = %project_root_label,
            "skipping startup auto-rebuild because only inactive worktree scopes require repair"
        );
        return None;
    }
    tracing::info!(
        target: "gwt::index",
        worktree = %project_root_label,
        target_count = targets.len(),
        "kicking auto-rebuild orchestrator after repair_required status"
    );
    gwt::auto_repair_unhealthy_targets(
        project_root,
        initial_status,
        targets,
        ServiceBackedRebuildSpawner::with_default_runner(service),
        final_status_provider,
        event_sink,
    )
}

struct InFlightGuard {
    in_flight: Arc<Mutex<HashSet<IndexInFlightKey>>>,
    key: IndexInFlightKey,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut in_flight) = self.in_flight.lock() {
            in_flight.remove(&self.key);
        }
    }
}

fn normalize_project_root(project_root: &Path) -> PathBuf {
    dunce::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf())
}

fn cached_aggregate_status_probe(project_root: &Path) -> gwt::ProjectIndexStatusView {
    gwt::global_aggregated_status_cache()
        .get_or_compute(project_root, gwt::aggregate_project_index_status_for_path)
}

fn current_worktree_status_probe(project_root: &Path) -> gwt::ProjectIndexStatusView {
    gwt::aggregate_current_worktree_index_status_for_path(project_root)
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        },
        time::Duration,
    };

    use tempfile::tempdir;

    use crate::{app_runtime::AppEventProxy, UserEvent};

    use super::IndexRebuildScope;

    fn wait_for_project_status(
        events: &Arc<Mutex<Vec<UserEvent>>>,
        expected_project_root: &str,
        expected_state: gwt::ProjectIndexStatusState,
    ) -> gwt::ProjectIndexStatusView {
        for _ in 0..100 {
            let recorded = events.lock().expect("events");
            if let Some(status) = recorded.iter().find_map(|event| match event {
                UserEvent::ProjectIndexStatus {
                    project_root,
                    status,
                } if project_root == expected_project_root && status.state == expected_state => {
                    Some(status.clone())
                }
                _ => None,
            }) {
                return status;
            }
            drop(recorded);
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("timed out waiting for project index status");
    }

    fn wait_for_project_status_detail(
        events: &Arc<Mutex<Vec<UserEvent>>>,
        expected_project_root: &str,
        expected_detail: &str,
    ) -> gwt::ProjectIndexStatusView {
        for _ in 0..100 {
            let recorded = events.lock().expect("events");
            if let Some(status) = recorded.iter().find_map(|event| match event {
                UserEvent::ProjectIndexStatus {
                    project_root,
                    status,
                } if project_root == expected_project_root && status.detail == expected_detail => {
                    Some(status.clone())
                }
                _ => None,
            }) {
                return status;
            }
            drop(recorded);
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("timed out waiting for project index status detail {expected_detail}");
    }

    #[test]
    fn duplicate_background_bootstrap_requests_for_same_project_are_coalesced() {
        let service = super::ProjectIndexBootstrapService::new_for_test();
        let temp = tempdir().expect("tempdir");
        let expected_project_root = dunce::canonicalize(temp.path())
            .unwrap_or_else(|_| temp.path().to_path_buf())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let call_count = Arc::new(AtomicUsize::new(0));
        let first_call_count = call_count.clone();

        let first = service.spawn_with(
            proxy.clone(),
            temp.path().to_path_buf(),
            move |_project_root: &Path| {
                first_call_count.fetch_add(1, Ordering::SeqCst);
                started_tx.send(()).expect("signal bootstrap start");
                release_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release bootstrap");
                Ok(())
            },
            |_project_root| {
                gwt::ProjectIndexStatusView::new(gwt::ProjectIndexStatusState::Ready, "ready")
            },
        );
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("background bootstrap should start");

        let second = service.spawn_with(
            proxy,
            temp.path().to_path_buf(),
            |_project_root| unreachable!("duplicate bootstrap should not run"),
            |_project_root| unreachable!("duplicate status probe should not run"),
        );

        assert_eq!(first, super::ProjectIndexBootstrapRequest::Spawned);
        assert_eq!(second, super::ProjectIndexBootstrapRequest::AlreadyRunning);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        release_tx.send(()).expect("release bootstrap");
        let status = wait_for_project_status(
            &events,
            &expected_project_root,
            gwt::ProjectIndexStatusState::Ready,
        );
        assert_eq!(status.detail, "ready");
    }

    // SPEC-2359 W-17 (FR-400): a reconnect storm replays frontend_ready on
    // every re-established WebSocket; each one used to restart the whole
    // bootstrap + status probe. Within the cooldown after a completed
    // bootstrap, repeat requests must reuse the cached status instead.
    #[test]
    fn completed_bootstrap_is_not_restarted_within_cooldown() {
        let service = super::ProjectIndexBootstrapService::new_for_test_with_bootstrap_cooldown(
            Duration::from_secs(60),
        );
        let temp = tempdir().expect("tempdir");
        let expected_project_root = dunce::canonicalize(temp.path())
            .unwrap_or_else(|_| temp.path().to_path_buf())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();
        let call_count = Arc::new(AtomicUsize::new(0));
        let first_call_count = call_count.clone();

        let first = service.spawn_with(
            proxy.clone(),
            temp.path().to_path_buf(),
            move |_project_root: &Path| {
                first_call_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_project_root| {
                gwt::ProjectIndexStatusView::new(gwt::ProjectIndexStatusState::Ready, "ready")
            },
        );
        assert_eq!(first, super::ProjectIndexBootstrapRequest::Spawned);
        let status = wait_for_project_status(
            &events,
            &expected_project_root,
            gwt::ProjectIndexStatusState::Ready,
        );
        assert_eq!(status.detail, "ready");

        // Reconnect-storm replay: another frontend_ready right after the
        // bootstrap completed.
        let second = service.spawn_with(
            proxy,
            temp.path().to_path_buf(),
            |_project_root| -> Result<(), String> {
                unreachable!("bootstrap must not restart within the cooldown")
            },
            |_project_root| -> gwt::ProjectIndexStatusView {
                unreachable!("status probe must not restart within the cooldown")
            },
        );

        assert_eq!(
            second,
            super::ProjectIndexBootstrapRequest::SkippedFresh,
            "repeat request within the cooldown is skipped"
        );
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // The cached status is re-emitted so a freshly-loaded page still
        // populates its status cell without a new sweep.
        let ready_count = events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    UserEvent::ProjectIndexStatus { project_root, status }
                        if project_root == &expected_project_root && status.detail == "ready"
                )
            })
            .count();
        assert_eq!(ready_count, 2, "cached status replayed to the new client");
    }

    #[test]
    fn full_status_refresh_retries_after_startup_bootstrap_coalesces() {
        let service = super::ProjectIndexBootstrapService::new_for_test();
        let temp = tempdir().expect("tempdir");
        let expected_project_root = dunce::canonicalize(temp.path())
            .unwrap_or_else(|_| temp.path().to_path_buf())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();
        let (startup_started_tx, startup_started_rx) = mpsc::channel();
        let (release_startup_tx, release_startup_rx) = mpsc::channel();
        let (full_probe_tx, full_probe_rx) = mpsc::channel();
        let full_probe_calls = Arc::new(AtomicUsize::new(0));
        let full_probe_calls_for_closure = full_probe_calls.clone();

        let startup = service.spawn_with(
            proxy.clone(),
            temp.path().to_path_buf(),
            move |_project_root: &Path| {
                startup_started_tx
                    .send(())
                    .expect("signal startup bootstrap start");
                release_startup_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release startup bootstrap");
                Ok(())
            },
            |_project_root| {
                gwt::ProjectIndexStatusView::new(
                    gwt::ProjectIndexStatusState::Ready,
                    "startup current",
                )
            },
        );
        assert_eq!(startup, super::ProjectIndexBootstrapRequest::Spawned);
        startup_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("startup bootstrap should hold the in-flight key");

        let full = service.spawn_full_status_refresh_with_retry(
            proxy.clone(),
            temp.path().to_path_buf(),
            Arc::new(|_project_root: &Path| Ok(())),
            Arc::new(move |_project_root: &Path| {
                full_probe_calls_for_closure.fetch_add(1, Ordering::SeqCst);
                full_probe_tx.send(()).expect("signal full probe");
                gwt::ProjectIndexStatusView::new(gwt::ProjectIndexStatusState::Ready, "full table")
            }),
            Duration::from_millis(5),
        );
        assert_eq!(full, super::ProjectIndexBootstrapRequest::AlreadyRunning);
        assert!(
            full_probe_rx
                .recv_timeout(Duration::from_millis(50))
                .is_err(),
            "full status probe must not run until the startup bootstrap releases"
        );

        release_startup_tx
            .send(())
            .expect("release startup bootstrap");
        let startup_status =
            wait_for_project_status_detail(&events, &expected_project_root, "startup current");
        assert_eq!(startup_status.state, gwt::ProjectIndexStatusState::Ready);
        full_probe_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("queued full status probe should run after startup");
        let full_status =
            wait_for_project_status_detail(&events, &expected_project_root, "full table");
        assert_eq!(full_status.state, gwt::ProjectIndexStatusState::Ready);
        assert_eq!(full_probe_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn duplicate_full_status_refresh_requests_collapse_without_queued_second_probe() {
        let service = super::ProjectIndexBootstrapService::new_for_test();
        let temp = tempdir().expect("tempdir");
        let expected_project_root = dunce::canonicalize(temp.path())
            .unwrap_or_else(|_| temp.path().to_path_buf())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();
        let (full_started_tx, full_started_rx) = mpsc::channel();
        let (release_full_tx, release_full_rx) = mpsc::channel();
        let release_full_rx = Arc::new(Mutex::new(release_full_rx));
        let full_probe_calls = Arc::new(AtomicUsize::new(0));
        let first_probe_calls = full_probe_calls.clone();
        let release_full_rx_for_closure = release_full_rx.clone();

        let first = service.spawn_full_status_refresh_with_retry(
            proxy.clone(),
            temp.path().to_path_buf(),
            Arc::new(move |_project_root: &Path| {
                full_started_tx.send(()).expect("signal full refresh start");
                release_full_rx_for_closure
                    .lock()
                    .expect("release receiver")
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release full refresh");
                Ok(())
            }),
            Arc::new(move |_project_root: &Path| {
                first_probe_calls.fetch_add(1, Ordering::SeqCst);
                gwt::ProjectIndexStatusView::new(gwt::ProjectIndexStatusState::Ready, "full table")
            }),
            Duration::from_millis(5),
        );
        assert_eq!(first, super::ProjectIndexBootstrapRequest::Spawned);
        full_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first full refresh should hold the in-flight slot");

        let duplicate_probe_calls = full_probe_calls.clone();
        let duplicate = service.spawn_full_status_refresh_with_retry(
            proxy.clone(),
            temp.path().to_path_buf(),
            Arc::new(|_project_root: &Path| Ok(())),
            Arc::new(move |_project_root: &Path| {
                duplicate_probe_calls.fetch_add(1, Ordering::SeqCst);
                gwt::ProjectIndexStatusView::new(
                    gwt::ProjectIndexStatusState::Ready,
                    "duplicate full table",
                )
            }),
            Duration::from_millis(5),
        );
        assert_eq!(
            duplicate,
            super::ProjectIndexBootstrapRequest::AlreadyRunning
        );

        release_full_tx.send(()).expect("release full refresh");
        let full_status =
            wait_for_project_status_detail(&events, &expected_project_root, "full table");
        assert_eq!(full_status.state, gwt::ProjectIndexStatusState::Ready);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            full_probe_calls.load(Ordering::SeqCst),
            1,
            "duplicate Settings.Index refresh requests must collapse into the in-flight full refresh"
        );
    }

    #[test]
    fn repeated_full_status_refresh_inside_cooldown_replays_cached_status_without_reprobing() {
        let service = super::ProjectIndexBootstrapService::new_for_test_with_full_status_cooldown(
            Duration::from_secs(60),
        );
        let temp = tempdir().expect("tempdir");
        let expected_project_root = dunce::canonicalize(temp.path())
            .unwrap_or_else(|_| temp.path().to_path_buf())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();
        let status_probe_calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let expected_request = if status_probe_calls.load(Ordering::SeqCst) == 0 {
                super::ProjectIndexBootstrapRequest::Spawned
            } else {
                super::ProjectIndexBootstrapRequest::AlreadyRunning
            };
            let status_probe_calls_for_closure = status_probe_calls.clone();
            let request = service.spawn_full_status_refresh_with_retry(
                proxy.clone(),
                temp.path().to_path_buf(),
                Arc::new(|_project_root: &Path| Ok(())),
                Arc::new(move |_project_root: &Path| {
                    status_probe_calls_for_closure.fetch_add(1, Ordering::SeqCst);
                    gwt::ProjectIndexStatusView::new(
                        gwt::ProjectIndexStatusState::Ready,
                        "unchanged full table",
                    )
                }),
                Duration::from_millis(5),
            );
            assert_eq!(request, expected_request);
            wait_for_project_status_detail(&events, &expected_project_root, "unchanged full table");
            for _ in 0..100 {
                if service.full_status_is_idle_for_test(temp.path()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        assert_eq!(
            status_probe_calls.load(Ordering::SeqCst),
            1,
            "a repeated full-status refresh inside the cooldown must not start another aggregate status probe"
        );
        let recorded = events.lock().expect("events");
        let full_status_events = recorded
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    UserEvent::ProjectIndexStatus { project_root, status }
                        if project_root == &expected_project_root
                            && status.detail == "unchanged full table"
                )
            })
            .count();
        assert_eq!(
            full_status_events, 2,
            "explicit Settings.Index refreshes inside cooldown should replay the cached full status"
        );
    }

    #[test]
    fn invalidating_full_status_cache_allows_next_refresh_after_rebuild() {
        let service = super::ProjectIndexBootstrapService::new_for_test_with_full_status_cooldown(
            Duration::from_secs(60),
        );
        let temp = tempdir().expect("tempdir");
        let expected_project_root = dunce::canonicalize(temp.path())
            .unwrap_or_else(|_| temp.path().to_path_buf())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();
        let status_probe_calls = Arc::new(AtomicUsize::new(0));

        for detail in ["first full table", "second full table"] {
            let calls = status_probe_calls.clone();
            let detail = detail.to_string();
            let detail_for_probe = detail.clone();
            let request = service.spawn_full_status_refresh_with_retry(
                proxy.clone(),
                temp.path().to_path_buf(),
                Arc::new(|_project_root: &Path| Ok(())),
                Arc::new(move |_project_root: &Path| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    gwt::ProjectIndexStatusView::new(
                        gwt::ProjectIndexStatusState::Ready,
                        &detail_for_probe,
                    )
                }),
                Duration::from_millis(5),
            );
            assert_eq!(request, super::ProjectIndexBootstrapRequest::Spawned);
            wait_for_project_status_detail(&events, &expected_project_root, &detail);
            for _ in 0..100 {
                if service.full_status_is_idle_for_test(temp.path()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            service.invalidate_full_status_for_test(temp.path());
        }

        assert_eq!(
            status_probe_calls.load(Ordering::SeqCst),
            2,
            "rebuild/repair invalidation must let the next full-status refresh probe again"
        );
    }

    #[test]
    fn failed_background_bootstrap_reports_error_and_releases_in_flight_slot() {
        let service = super::ProjectIndexBootstrapService::new_for_test();
        let temp = tempdir().expect("tempdir");
        let expected_project_root = dunce::canonicalize(temp.path())
            .unwrap_or_else(|_| temp.path().to_path_buf())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();

        let first = service.spawn_with(
            proxy.clone(),
            temp.path().to_path_buf(),
            |_project_root: &Path| Err("synthetic bootstrap failure".to_string()),
            |_project_root| unreachable!("status probe should not run after bootstrap failure"),
        );

        assert_eq!(first, super::ProjectIndexBootstrapRequest::Spawned);
        let status = wait_for_project_status(
            &events,
            &expected_project_root,
            gwt::ProjectIndexStatusState::Error,
        );
        assert!(status.detail.contains("synthetic bootstrap failure"));

        let mut retry = super::ProjectIndexBootstrapRequest::AlreadyRunning;
        for _ in 0..100 {
            retry = service.spawn_with(
                proxy.clone(),
                temp.path().to_path_buf(),
                |_project_root: &Path| Ok(()),
                |_project_root| {
                    gwt::ProjectIndexStatusView::new(
                        gwt::ProjectIndexStatusState::Ready,
                        "retry ready",
                    )
                },
            );
            if retry == super::ProjectIndexBootstrapRequest::Spawned {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(retry, super::ProjectIndexBootstrapRequest::Spawned);
        let status = wait_for_project_status(
            &events,
            &expected_project_root,
            gwt::ProjectIndexStatusState::Ready,
        );
        assert_eq!(status.detail, "retry ready");
    }

    #[test]
    fn spawn_per_cell_rebuild_emits_repairing_then_final_status_via_proxy() {
        // SPEC-1939 T-IDX-109 (subset): exercise the per-cell IPC path
        // end-to-end by injecting a fake runner + final-status provider so
        // the test does not invoke real Python. The recorded proxy events
        // must follow `Repairing(0/1)` -> final, mirroring what the
        // frontend `setIndexStatus` consumes from WebSocket.
        let service = super::ProjectIndexBootstrapService::new_for_test();
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().to_path_buf();
        // spawn_per_cell_rebuild_with canonicalises the project root so
        // proxy events share a key with the bootstrap path.
        let project_root_label = dunce::canonicalize(&project_root)
            .unwrap_or_else(|_| project_root.clone())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();

        let runner_calls = Arc::new(AtomicUsize::new(0));
        let runner_calls_handle = runner_calls.clone();
        let rebuild_runner: Arc<gwt::IndexRebuildRunnerFn> =
            Arc::new(move |_root, scope, worktree_hash| {
                assert_eq!(scope, IndexRebuildScope::Files);
                assert_eq!(worktree_hash, Some("wtAhash"));
                runner_calls_handle.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });
        let final_status_provider: Arc<
            dyn Fn(&Path) -> gwt::ProjectIndexStatusView + Send + Sync + 'static,
        > = Arc::new(|_path| {
            gwt::ProjectIndexStatusView::new(
                gwt::ProjectIndexStatusState::Ready,
                "ready after IPC rebuild",
            )
        });

        let request = super::spawn_per_cell_rebuild_with(
            service,
            proxy,
            project_root.clone(),
            IndexRebuildScope::Files,
            Some("wtAhash".to_string()),
            rebuild_runner,
            final_status_provider,
        );
        assert_eq!(request, super::ProjectIndexBootstrapRequest::Spawned);

        let final_view = wait_for_project_status(
            &events,
            &project_root_label,
            gwt::ProjectIndexStatusState::Ready,
        );
        assert_eq!(final_view.detail, "ready after IPC rebuild");
        assert_eq!(runner_calls.load(Ordering::SeqCst), 1);

        // The transient Repairing event must have been emitted before the
        // Ready event.
        let recorded = events.lock().expect("events");
        let mut saw_repairing_before_ready = false;
        for event in recorded.iter() {
            if let UserEvent::ProjectIndexStatus {
                project_root,
                status,
            } = event
            {
                if project_root != &project_root_label {
                    continue;
                }
                if status.state == gwt::ProjectIndexStatusState::Repairing {
                    saw_repairing_before_ready = true;
                    let progress = status.progress.expect("progress on repairing");
                    assert_eq!(progress.scopes_total, 1);
                }
                if status.state == gwt::ProjectIndexStatusState::Ready {
                    assert!(
                        saw_repairing_before_ready,
                        "Ready must follow at least one Repairing event"
                    );
                    break;
                }
            }
        }
    }

    #[test]
    fn spawn_per_cell_rebuild_emits_error_state_when_runner_fails() {
        // SPEC-1939 T-IDX-110 (subset): runner failure surfaces as `error`
        // with the rebuild reason, mirroring what the badge / Settings.Index
        // shows on auto-rebuild failure.
        let service = super::ProjectIndexBootstrapService::new_for_test();
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().to_path_buf();
        // spawn_per_cell_rebuild_with canonicalises the project root so
        // proxy events share a key with the bootstrap path.
        let project_root_label = dunce::canonicalize(&project_root)
            .unwrap_or_else(|_| project_root.clone())
            .display()
            .to_string();
        let (proxy, events) = AppEventProxy::stub();

        let rebuild_runner: Arc<gwt::IndexRebuildRunnerFn> =
            Arc::new(|_root, _scope, _worktree_hash| Err("synthetic IPC failure".to_string()));
        let final_status_provider: Arc<
            dyn Fn(&Path) -> gwt::ProjectIndexStatusView + Send + Sync + 'static,
        > = Arc::new(|_path| {
            gwt::ProjectIndexStatusView::new(
                gwt::ProjectIndexStatusState::Ready,
                "should not be used",
            )
        });

        let request = super::spawn_per_cell_rebuild_with(
            service,
            proxy,
            project_root.clone(),
            IndexRebuildScope::Specs,
            None,
            rebuild_runner,
            final_status_provider,
        );
        assert_eq!(request, super::ProjectIndexBootstrapRequest::Spawned);

        let final_view = wait_for_project_status(
            &events,
            &project_root_label,
            gwt::ProjectIndexStatusState::Error,
        );
        assert!(
            final_view.detail.contains("synthetic IPC failure"),
            "detail should carry the failure reason: {}",
            final_view.detail
        );
    }

    #[test]
    fn rebuild_for_same_cell_is_coalesced_while_other_keys_run_in_parallel() {
        // Long timeouts intentionally tolerate slow CI hosts: every blocking
        // recv waits up to 60s, every "completed" channel waits up to 30s,
        // and the post-release retry loop polls for up to 30s. Real failures
        // surface as in-flight semantics regressions, not timing flakes.
        const RECV_TIMEOUT: Duration = Duration::from_secs(60);
        const DONE_TIMEOUT: Duration = Duration::from_secs(30);
        const RETRY_TIMEOUT: Duration = Duration::from_secs(30);

        let service = super::ProjectIndexBootstrapService::new_for_test();
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().to_path_buf();
        let (block_files_tx, block_files_rx) = mpsc::channel();
        let (block_specs_tx, block_specs_rx) = mpsc::channel();
        let (block_bootstrap_tx, block_bootstrap_rx) = mpsc::channel();
        // Per-thread "exited" signals let the test wait for InFlightGuard
        // drop deterministically instead of polling on the in-flight set.
        let (files_exited_tx, files_exited_rx) = mpsc::channel();
        let (specs_exited_tx, specs_exited_rx) = mpsc::channel();
        let (bootstrap_exited_tx, bootstrap_exited_rx) = mpsc::channel();
        let files_calls = Arc::new(AtomicUsize::new(0));
        let specs_calls = Arc::new(AtomicUsize::new(0));
        let bootstrap_calls = Arc::new(AtomicUsize::new(0));

        let files_calls_handle = files_calls.clone();
        let first_files = service.spawn_rebuild_with(
            project_root.clone(),
            IndexRebuildScope::Files,
            Some("wtA".to_string()),
            move || {
                files_calls_handle.fetch_add(1, Ordering::SeqCst);
                block_files_rx
                    .recv_timeout(RECV_TIMEOUT)
                    .expect("release files");
                let _ = files_exited_tx.send(());
            },
        );

        // Same key: should coalesce while the first task is still running.
        let duplicate_files = service.spawn_rebuild_with(
            project_root.clone(),
            IndexRebuildScope::Files,
            Some("wtA".to_string()),
            || unreachable!("duplicate cell rebuild should not run"),
        );

        // Different worktree on the same scope: must proceed in parallel.
        let other_worktree_calls = Arc::new(AtomicUsize::new(0));
        let other_worktree_calls_handle = other_worktree_calls.clone();
        let (other_worktree_done_tx, other_worktree_done_rx) = mpsc::channel();
        let other_worktree = service.spawn_rebuild_with(
            project_root.clone(),
            IndexRebuildScope::Files,
            Some("wtB".to_string()),
            move || {
                other_worktree_calls_handle.fetch_add(1, Ordering::SeqCst);
                other_worktree_done_tx.send(()).expect("signal wtB done");
            },
        );

        // Different scope: must proceed in parallel.
        let specs_calls_handle = specs_calls.clone();
        let specs = service.spawn_rebuild_with(
            project_root.clone(),
            IndexRebuildScope::Specs,
            None,
            move || {
                specs_calls_handle.fetch_add(1, Ordering::SeqCst);
                block_specs_rx
                    .recv_timeout(RECV_TIMEOUT)
                    .expect("release specs");
                let _ = specs_exited_tx.send(());
            },
        );

        // Bootstrap for the same project must also proceed in parallel.
        let bootstrap_calls_handle = bootstrap_calls.clone();
        let (proxy, _events) = AppEventProxy::stub();
        let bootstrap = service.spawn_with(
            proxy,
            project_root.clone(),
            move |_project_root: &Path| {
                bootstrap_calls_handle.fetch_add(1, Ordering::SeqCst);
                block_bootstrap_rx
                    .recv_timeout(RECV_TIMEOUT)
                    .expect("release bootstrap");
                let _ = bootstrap_exited_tx.send(());
                Ok(())
            },
            |_project_root| {
                gwt::ProjectIndexStatusView::new(gwt::ProjectIndexStatusState::Ready, "ready")
            },
        );

        assert_eq!(first_files, super::ProjectIndexBootstrapRequest::Spawned);
        assert_eq!(
            duplicate_files,
            super::ProjectIndexBootstrapRequest::AlreadyRunning
        );
        assert_eq!(other_worktree, super::ProjectIndexBootstrapRequest::Spawned);
        assert_eq!(specs, super::ProjectIndexBootstrapRequest::Spawned);
        assert_eq!(bootstrap, super::ProjectIndexBootstrapRequest::Spawned);

        // The wtB task is unblocked and should complete on its own.
        other_worktree_done_rx
            .recv_timeout(DONE_TIMEOUT)
            .expect("wtB rebuild should run in parallel");
        assert_eq!(other_worktree_calls.load(Ordering::SeqCst), 1);

        // Release the blocked tasks so threads exit cleanly. We then wait
        // for the explicit "exited" signal from each closure: by the time
        // those signals fire, the closures have returned and the
        // InFlightGuard drops are scheduled. Polling for the slot to be
        // released afterwards is bounded but typically immediate.
        block_files_tx.send(()).expect("release files");
        block_specs_tx.send(()).expect("release specs");
        block_bootstrap_tx.send(()).expect("release bootstrap");
        files_exited_rx
            .recv_timeout(DONE_TIMEOUT)
            .expect("files closure should exit");
        specs_exited_rx
            .recv_timeout(DONE_TIMEOUT)
            .expect("specs closure should exit");
        bootstrap_exited_rx
            .recv_timeout(DONE_TIMEOUT)
            .expect("bootstrap closure should exit");

        // After the first files rebuild completes, a new one can be queued.
        // The retry loop bounds wall time by RETRY_TIMEOUT to absorb any
        // residual gap between closure return and InFlightGuard drop.
        let started = std::time::Instant::now();
        let mut retry = super::ProjectIndexBootstrapRequest::AlreadyRunning;
        while started.elapsed() < RETRY_TIMEOUT {
            retry = service.spawn_rebuild_with(
                project_root.clone(),
                IndexRebuildScope::Files,
                Some("wtA".to_string()),
                || {},
            );
            if retry == super::ProjectIndexBootstrapRequest::Spawned {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(retry, super::ProjectIndexBootstrapRequest::Spawned);
        assert_eq!(files_calls.load(Ordering::SeqCst), 1);
        assert_eq!(specs_calls.load(Ordering::SeqCst), 1);
        assert_eq!(bootstrap_calls.load(Ordering::SeqCst), 1);
    }
}

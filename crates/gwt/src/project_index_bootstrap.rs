use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::Instant,
};

use crate::{app_runtime::AppEventProxy, UserEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectIndexBootstrapRequest {
    Spawned,
    AlreadyRunning,
    SpawnFailed,
}

#[derive(Clone, Default)]
pub(crate) struct ProjectIndexBootstrapService {
    in_flight: Arc<Mutex<HashSet<PathBuf>>>,
}

impl ProjectIndexBootstrapService {
    pub(crate) fn global() -> &'static Self {
        static SERVICE: OnceLock<ProjectIndexBootstrapService> = OnceLock::new();
        SERVICE.get_or_init(Self::default)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self::default()
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
            gwt::index_worker::project_index_status_for_path,
        )
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
        {
            let mut in_flight = self.in_flight.lock().expect("project index in-flight set");
            if !in_flight.insert(project_key.clone()) {
                tracing::debug!(
                    target: "gwt::index",
                    worktree = %project_root_label,
                    "project index bootstrap already running for worktree"
                );
                return ProjectIndexBootstrapRequest::AlreadyRunning;
            }
        }

        let in_flight = self.in_flight.clone();
        let project_key_for_thread = project_key.clone();
        let spawn_result = thread::Builder::new()
            .name("gwt-index-bootstrap".to_string())
            .spawn(move || {
                let _guard = InFlightBootstrapGuard {
                    in_flight,
                    project_key: project_key_for_thread.clone(),
                };
                let bootstrap_started = Instant::now();
                match bootstrap(&project_key_for_thread) {
                    Ok(()) => {
                        let bootstrap_elapsed_ms = bootstrap_started.elapsed().as_millis() as u64;
                        tracing::info!(
                            target: "gwt::index",
                            worktree = %project_root_label,
                            elapsed_ms = bootstrap_elapsed_ms,
                            "project index bootstrap completed in background"
                        );

                        let status_started = Instant::now();
                        let status = status_probe(&project_key_for_thread);
                        tracing::info!(
                            target: "gwt::index",
                            worktree = %project_root_label,
                            elapsed_ms = status_started.elapsed().as_millis() as u64,
                            state = %status.state,
                            "project index status refreshed after background bootstrap"
                        );
                        proxy.send(UserEvent::ProjectIndexStatus {
                            project_root: project_root_label,
                            status,
                        });
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
                            status: gwt::ProjectIndexStatusView {
                                state: gwt::ProjectIndexStatusState::Error,
                                detail: format!(
                                    "Project index bootstrap failed after {elapsed_ms} ms: {error}"
                                ),
                            },
                        });
                    }
                }
            });

        match spawn_result {
            Ok(_) => ProjectIndexBootstrapRequest::Spawned,
            Err(error) => {
                let mut in_flight = self.in_flight.lock().expect("project index in-flight set");
                in_flight.remove(&project_key);
                tracing::warn!(
                    target: "gwt::index",
                    error = %error,
                    "failed to spawn project index bootstrap background task"
                );
                ProjectIndexBootstrapRequest::SpawnFailed
            }
        }
    }
}

struct InFlightBootstrapGuard {
    in_flight: Arc<Mutex<HashSet<PathBuf>>>,
    project_key: PathBuf,
}

impl Drop for InFlightBootstrapGuard {
    fn drop(&mut self) {
        if let Ok(mut in_flight) = self.in_flight.lock() {
            in_flight.remove(&self.project_key);
        }
    }
}

fn normalize_project_root(project_root: &Path) -> PathBuf {
    dunce::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf())
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
            |_project_root| gwt::ProjectIndexStatusView {
                state: gwt::ProjectIndexStatusState::Ready,
                detail: "ready".to_string(),
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
                |_project_root| gwt::ProjectIndexStatusView {
                    state: gwt::ProjectIndexStatusState::Ready,
                    detail: "retry ready".to_string(),
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
}

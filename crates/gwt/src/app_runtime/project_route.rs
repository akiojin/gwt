//! Issue #4538: per-project URL routing (`/p/<repo-hash>`) and the local
//! `gwt open <path>` control request.
//!
//! A browser tab resolves its Project from the URL, so the runtime must map a
//! ProjectKey back to a root: open Projects first, then Recent entries. Recent
//! keys are computed by a blocking worker because repository identity reads
//! Git metadata; the tao thread only consults the cache. No filesystem scan is
//! used to guess a root from an unknown hash (SPEC #3287 EC-003).

use super::*;

/// Outcome delivered to a `POST /internal/projects/open` caller.
pub(crate) type ProjectOpenOutcome =
    Result<gwt_core::repo_hash::ProjectKey, ProjectOpenControlFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectOpenControlFailure {
    /// The path cannot be opened as a Project (HTTP 422).
    Rejected(String),
    /// The runtime could not complete this request (HTTP 503).
    Unavailable(String),
}

/// Single-use reply slot carried through `UserEvent`. `UserEvent` is `Clone`,
/// so the sender lives behind a shared take-once cell.
#[derive(Clone)]
pub(crate) struct ProjectOpenReply(
    Arc<Mutex<Option<tokio::sync::oneshot::Sender<ProjectOpenOutcome>>>>,
);

impl ProjectOpenReply {
    pub(crate) fn channel() -> (Self, tokio::sync::oneshot::Receiver<ProjectOpenOutcome>) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(sender)))), receiver)
    }

    pub(crate) fn send(&self, outcome: ProjectOpenOutcome) {
        let sender = self.0.lock().ok().and_then(|mut slot| slot.take());
        if let Some(sender) = sender {
            let _ = sender.send(outcome);
        }
    }
}

impl std::fmt::Debug for ProjectOpenReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProjectOpenReply")
    }
}

/// A `/p/<hash>` client that is waiting for Recent keys before the runtime
/// can decide between auto-open and not-found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectRouteRequest {
    pub(crate) client_id: String,
    pub(crate) project_key: gwt_core::repo_hash::ProjectKey,
}

#[derive(Debug, Clone)]
pub(crate) struct RecentProjectKeysResolved {
    pub(crate) keys: Vec<(PathBuf, gwt_core::repo_hash::ProjectKey)>,
    pub(crate) route: Option<ProjectRouteRequest>,
}

#[derive(Debug, Default)]
pub(crate) struct ProjectRouteState {
    /// Recent path → ProjectKey, filled by the resolution worker and by every
    /// committed open.
    recent_keys: HashMap<PathBuf, gwt_core::repo_hash::ProjectKey>,
    /// Control-open callers keyed by their project navigation request id.
    open_waiters: Vec<(u64, ProjectOpenReply)>,
}

impl AppRuntime {
    pub(crate) fn recent_project_key(
        &self,
        path: &Path,
    ) -> Option<&gwt_core::repo_hash::ProjectKey> {
        self.project_route.recent_keys.get(path)
    }

    pub(crate) fn remember_recent_project_key(
        &mut self,
        path: PathBuf,
        key: gwt_core::repo_hash::ProjectKey,
    ) {
        self.project_route.recent_keys.insert(path, key);
    }

    fn recent_paths_missing_keys(&self) -> Vec<PathBuf> {
        self.recent_projects
            .iter()
            .filter(|entry| !self.project_route.recent_keys.contains_key(&entry.path))
            .map(|entry| entry.path.clone())
            .collect()
    }

    fn recent_path_for_key(&self, key: &gwt_core::repo_hash::ProjectKey) -> Option<PathBuf> {
        self.recent_projects
            .iter()
            .find(|entry| self.project_route.recent_keys.get(&entry.path) == Some(key))
            .map(|entry| entry.path.clone())
    }

    /// Resolve every uncached Recent key off the tao thread. Returns `false`
    /// when nothing needed resolving.
    fn spawn_recent_project_key_resolution(&self, route: Option<ProjectRouteRequest>) -> bool {
        let paths = self.recent_paths_missing_keys();
        if paths.is_empty() {
            return false;
        }
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let keys = paths
                .into_iter()
                .map(|path| {
                    let key = gwt_core::paths::resolve_project_scope(&path).hash;
                    (path, key)
                })
                .collect();
            proxy.send(UserEvent::RecentProjectKeysResolved(
                RecentProjectKeysResolved { keys, route },
            ));
        });
        true
    }

    /// Hub hydration: Recent links need keys, so resolve the missing ones.
    pub(crate) fn ensure_recent_project_keys(&self) {
        self.spawn_recent_project_key_resolution(None);
    }

    /// `frontend_ready` from a `/p/<hash>` client whose Project is not open.
    pub(crate) fn unopened_project_route_events(
        &mut self,
        client_id: &str,
        key: &gwt_core::repo_hash::ProjectKey,
    ) -> Vec<OutboundEvent> {
        if let Some(path) = self.recent_path_for_key(key) {
            return self.open_project_path_events(path);
        }
        let route = ProjectRouteRequest {
            client_id: client_id.to_string(),
            project_key: key.clone(),
        };
        if self.spawn_recent_project_key_resolution(Some(route)) {
            return Vec::new();
        }
        vec![project_not_found_reply(client_id, key)]
    }

    pub(crate) fn handle_recent_project_keys_resolved(
        &mut self,
        resolved: RecentProjectKeysResolved,
    ) -> Vec<OutboundEvent> {
        self.project_route.recent_keys.extend(resolved.keys);
        let mut events = vec![self.hub_state_broadcast()];
        if let Some(route) = resolved.route {
            let already_open = self
                .project_contexts()
                .iter()
                .any(|context| context.project_key == route.project_key);
            if already_open {
                return events;
            }
            match self.recent_path_for_key(&route.project_key) {
                Some(path) => events.extend(self.open_project_path_events(path)),
                None => events.push(project_not_found_reply(
                    &route.client_id,
                    &route.project_key,
                )),
            }
        }
        events
    }

    /// `POST /internal/projects/open`: open `path` through the async
    /// prepare/commit route and answer once the commit lands.
    pub(crate) fn control_project_open_events(
        &mut self,
        path: PathBuf,
        reply: ProjectOpenReply,
    ) -> Vec<OutboundEvent> {
        let events = self.open_project_path_events(path);
        match self.pending_project_navigation.as_ref() {
            Some(request) if events.is_empty() => {
                self.project_route.open_waiters.push((request.id, reply));
            }
            _ => reply.send(Err(ProjectOpenControlFailure::Unavailable(
                "project open could not start".to_string(),
            ))),
        }
        events
    }

    /// Settle the control-open caller (if any) of a prepared navigation.
    pub(crate) fn settle_project_open_waiter(
        &mut self,
        request_id: u64,
        outcome: ProjectOpenOutcome,
    ) {
        let waiters = std::mem::take(&mut self.project_route.open_waiters);
        for (id, reply) in waiters {
            if id == request_id {
                reply.send(outcome.clone());
            } else {
                self.project_route.open_waiters.push((id, reply));
            }
        }
    }
}

fn project_not_found_reply(
    client_id: &str,
    key: &gwt_core::repo_hash::ProjectKey,
) -> OutboundEvent {
    OutboundEvent::reply(
        client_id,
        BackendEvent::ProjectNotFound {
            project_key: key.to_string(),
        },
    )
}

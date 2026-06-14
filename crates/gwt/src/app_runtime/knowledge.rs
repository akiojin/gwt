//! Knowledge bridge / project-index search handlers split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 1).
//!
//! Owns:
//! - The frontend request payloads ([`KnowledgeSearchRequest`],
//!   [`KnowledgeLoadRequest`], [`ProjectIndexSearchRequest`]) and the
//!   off-thread task payloads ([`KnowledgeRefreshTask`],
//!   [`KnowledgeSearchTask`], [`ProjectIndexSearchTask`])
//! - [`AppRuntime::load_knowledge_bridge_events`] /
//!   [`AppRuntime::search_knowledge_bridge_events`] /
//!   [`AppRuntime::search_project_index_events`] /
//!   [`AppRuntime::update_knowledge_bridge_phase_events`] — Knowledge window
//!   loaders and the blocking-task spawns backing them
//! - [`AppRuntime::rebuild_index_cell_events`] /
//!   [`AppRuntime::refresh_index_status_events`] — Settings.Index per-cell
//!   rebuild and health refresh (SPEC-1939)
//!
//! Behavior-preserving move: the cache/search implementations stay in
//! `crate::knowledge_bridge` / `crate::index_search`.

use std::path::PathBuf;

use gwt::KnowledgeKind;

use super::{
    knowledge_kind_for_preset, load_knowledge_bridge, AppRuntime, BackendEvent, OutboundEvent,
    UserEvent, WindowPreset,
};

pub struct KnowledgeSearchRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: KnowledgeKind,
    pub(crate) query: &'a str,
    pub(crate) request_id: u64,
    pub(crate) selected_number: Option<u64>,
}

pub struct KnowledgeLoadRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: KnowledgeKind,
    pub(crate) request_id: Option<u64>,
    pub(crate) selected_number: Option<u64>,
    pub(crate) refresh: bool,
}

pub struct ProjectIndexSearchRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) query: &'a str,
    pub(crate) request_id: u64,
    pub(crate) scopes: Vec<gwt::IndexSearchScope>,
    pub(crate) worktree_hash: Option<String>,
    pub(crate) match_mode: gwt::IndexSearchMatchMode,
}

pub(super) struct KnowledgeRefreshTask {
    pub(super) client_id: String,
    pub(super) id: String,
    pub(super) project_root: PathBuf,
    pub(super) kind: KnowledgeKind,
    pub(super) request_id: Option<u64>,
    pub(super) selected_number: Option<u64>,
    pub(super) force: bool,
}

struct KnowledgeSearchTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    kind: KnowledgeKind,
    query: String,
    request_id: u64,
    selected_number: Option<u64>,
}

struct ProjectIndexSearchTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    query: String,
    request_id: u64,
    scopes: Vec<gwt::IndexSearchScope>,
    worktree_hash: Option<String>,
    match_mode: gwt::IndexSearchMatchMode,
}

pub(super) fn knowledge_error_event(
    id: impl Into<String>,
    kind: KnowledgeKind,
    message: impl Into<String>,
    request_id: Option<u64>,
    query: Option<String>,
) -> BackendEvent {
    BackendEvent::KnowledgeError {
        id: id.into(),
        knowledge_kind: kind,
        request_id,
        query,
        message: message.into(),
    }
}

fn knowledge_phase_update_error_event(
    id: impl Into<String>,
    request_id: u64,
    issue_number: u64,
    message: impl Into<String>,
) -> BackendEvent {
    BackendEvent::KnowledgeBridgePhaseUpdated {
        id: id.into(),
        request_id,
        issue_number,
        result: gwt::protocol::KnowledgePhaseUpdateResult::Error {
            message: message.into(),
        },
    }
}

fn knowledge_view_events(
    client_id: String,
    id: String,
    kind: KnowledgeKind,
    request_id: Option<u64>,
    view: gwt::KnowledgeBridgeView,
) -> Vec<OutboundEvent> {
    vec![
        OutboundEvent::reply(
            client_id.clone(),
            BackendEvent::KnowledgeEntries {
                id: id.clone(),
                knowledge_kind: kind,
                request_id,
                entries: view.entries,
                selected_number: view.selected_number,
                empty_message: view.empty_message,
                refresh_enabled: view.refresh_enabled,
            },
        ),
        OutboundEvent::reply(
            client_id,
            BackendEvent::KnowledgeDetail {
                id,
                knowledge_kind: kind,
                request_id,
                detail: view.detail,
            },
        ),
    ]
}

impl AppRuntime {
    pub(crate) fn load_knowledge_bridge_events(
        &self,
        client_id: &str,
        request: KnowledgeLoadRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let kind = request.kind;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Window not found", request.request_id, None),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Project tab not found", request.request_id, None),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Window not found", request.request_id, None),
            )];
        };
        if knowledge_kind_for_preset(window.preset) != Some(kind) {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window is not a knowledge bridge",
                    request.request_id,
                    None,
                ),
            )];
        }

        if request.refresh {
            self.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
                client_id: client_id.to_string(),
                id: id.to_string(),
                project_root: tab.project_root.clone(),
                kind,
                request_id: request.request_id,
                selected_number: request.selected_number,
                force: true,
            });
            return Vec::new();
        }

        match load_knowledge_bridge(&tab.project_root, kind, request.selected_number, false) {
            Ok(view) => {
                if request.request_id.is_some() && view.refresh_enabled {
                    self.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
                        client_id: client_id.to_string(),
                        id: id.to_string(),
                        project_root: tab.project_root.clone(),
                        kind,
                        request_id: request.request_id,
                        selected_number: request.selected_number,
                        force: false,
                    });
                }
                knowledge_view_events(
                    client_id.to_string(),
                    id.to_string(),
                    kind,
                    request.request_id,
                    view,
                )
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, error, request.request_id, None),
            )],
        }
    }

    pub(super) fn spawn_knowledge_bridge_refresh(&self, task: KnowledgeRefreshTask) {
        let KnowledgeRefreshTask {
            client_id,
            id,
            project_root,
            kind,
            request_id,
            selected_number,
            force,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let refreshed = match gwt::refresh_knowledge_bridge_cache(&project_root, force) {
                Ok(refreshed) => refreshed,
                Err(error) => {
                    if force {
                        proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                            client_id,
                            knowledge_error_event(id, kind, error, request_id, None),
                        )]));
                    }
                    return;
                }
            };
            if !force && !refreshed {
                return;
            }
            let event =
                match gwt::load_knowledge_bridge(&project_root, kind, selected_number, false) {
                    Ok(view) => knowledge_view_events(client_id, id, kind, request_id, view),
                    Err(error) => vec![OutboundEvent::reply(
                        client_id,
                        knowledge_error_event(id, kind, error, request_id, None),
                    )],
                };
            proxy.send(UserEvent::Dispatch(event));
        });
    }

    pub(crate) fn search_knowledge_bridge_events(
        &self,
        client_id: &str,
        request: KnowledgeSearchRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let kind = request.kind;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Project tab not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        if knowledge_kind_for_preset(window.preset) != Some(kind) {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window is not a knowledge bridge",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        }

        self.spawn_knowledge_bridge_search(KnowledgeSearchTask {
            client_id: client_id.to_string(),
            id: id.to_string(),
            project_root: tab.project_root.clone(),
            kind,
            query: request.query.to_string(),
            request_id: request.request_id,
            selected_number: request.selected_number,
        });
        Vec::new()
    }

    fn spawn_knowledge_bridge_search(&self, task: KnowledgeSearchTask) {
        let KnowledgeSearchTask {
            client_id,
            id,
            project_root,
            kind,
            query,
            request_id,
            selected_number,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event =
                match gwt::search_knowledge_bridge(&project_root, kind, &query, selected_number) {
                    Ok(view) => BackendEvent::KnowledgeSearchResults {
                        id: id.clone(),
                        knowledge_kind: kind,
                        query: query.clone(),
                        request_id,
                        entries: view.entries,
                        selected_number: view.selected_number,
                        empty_message: view.empty_message,
                        refresh_enabled: view.refresh_enabled,
                    },
                    Err(error) => {
                        knowledge_error_event(id, kind, error, Some(request_id), Some(query))
                    }
                };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    pub(crate) fn search_project_index_events(
        &self,
        client_id: &str,
        request: ProjectIndexSearchRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Index {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window is not an Index surface".to_string(),
                },
            )];
        }

        self.spawn_project_index_search(ProjectIndexSearchTask {
            client_id: client_id.to_string(),
            id: id.to_string(),
            project_root: tab.project_root.clone(),
            query: request.query.to_string(),
            request_id: request.request_id,
            scopes: request.scopes,
            worktree_hash: request.worktree_hash,
            match_mode: request.match_mode,
        });
        Vec::new()
    }

    fn spawn_project_index_search(&self, task: ProjectIndexSearchTask) {
        let ProjectIndexSearchTask {
            client_id,
            id,
            project_root,
            query,
            request_id,
            scopes,
            worktree_hash,
            match_mode,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = match gwt::search_project_index(
                &project_root,
                &query,
                &scopes,
                worktree_hash.as_deref(),
                match_mode,
                // GUI interactive search: the watcher owns index builds.
                false,
            ) {
                Ok(outcome) => BackendEvent::ProjectIndexSearchResults {
                    id: id.clone(),
                    query: query.clone(),
                    request_id,
                    results: outcome.results,
                    suggestions: outcome.suggestions,
                },
                Err(error) => BackendEvent::ProjectIndexSearchError {
                    id: id.clone(),
                    query: query.clone(),
                    request_id,
                    message: error,
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    /// SPEC-2017 US-8 — Apply a Kanban phase change to the owning
    /// GitHub Issue. Validates that the target window is a knowledge
    /// bridge surface and dispatches a blocking task that calls
    /// `gwt::update_knowledge_phase`. The result is delivered as
    /// [`BackendEvent::KnowledgeBridgePhaseUpdated`] so the optimistic
    /// frontend UI can either confirm or rollback.
    pub(crate) fn update_knowledge_bridge_phase_events(
        &self,
        client_id: &str,
        id: &str,
        request_id: u64,
        issue_number: u64,
        target_phase: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window not found",
                ),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Project tab not found",
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window not found",
                ),
            )];
        };
        if knowledge_kind_for_preset(window.preset).is_none() {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window is not a knowledge bridge",
                ),
            )];
        }

        let proxy = self.proxy.clone();
        let client_id = client_id.to_string();
        let id_owned = id.to_string();
        let project_root = tab.project_root.clone();
        let target_phase = target_phase.map(str::to_string);
        self.blocking_tasks.spawn(move || {
            let event = match gwt::update_knowledge_phase(
                &project_root,
                issue_number,
                target_phase.as_deref(),
            ) {
                Ok(fresh_entry) => BackendEvent::KnowledgeBridgePhaseUpdated {
                    id: id_owned,
                    request_id,
                    issue_number,
                    result: gwt::protocol::KnowledgePhaseUpdateResult::Ok { fresh_entry },
                },
                Err(error) => BackendEvent::KnowledgeBridgePhaseUpdated {
                    id: id_owned,
                    request_id,
                    issue_number,
                    result: gwt::protocol::KnowledgePhaseUpdateResult::Error { message: error },
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                &client_id, event,
            )]));
        });
        Vec::new()
    }

    /// SPEC-1939 US-5 / T-IDX-102: handle a per-cell rebuild request from the
    /// frontend. Spawns the rebuild via the global bootstrap service so the
    /// in-flight set is shared with the orchestrator and CLI.
    pub(crate) fn rebuild_index_cell_events(
        &self,
        project_root: String,
        scope: gwt::IndexRebuildScope,
        worktree_hash: Option<String>,
    ) -> Vec<OutboundEvent> {
        let project_root = std::path::PathBuf::from(project_root);
        let service =
            crate::project_index_bootstrap::ProjectIndexBootstrapService::global().clone();
        let _request = crate::project_index_bootstrap::spawn_per_cell_rebuild(
            service,
            self.proxy.clone(),
            project_root,
            scope,
            worktree_hash,
        );
        Vec::new()
    }

    /// Settings.Index requests the full all-worktree health table on demand.
    /// The startup path stays current-worktree only to avoid UI-visible CPU
    /// spikes on repositories with many active worktrees.
    pub(crate) fn refresh_index_status_events(&self, project_root: String) -> Vec<OutboundEvent> {
        let project_root = std::path::PathBuf::from(project_root);
        let service =
            crate::project_index_bootstrap::ProjectIndexBootstrapService::global().clone();
        let _request = service.spawn_full_status_refresh(self.proxy.clone(), project_root);
        Vec::new()
    }
}

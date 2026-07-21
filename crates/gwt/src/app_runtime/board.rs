//! Board post handler split out of `app_runtime/mod.rs` for SPEC-2077 Phase A
//! (arch-review handoff, 2026-05-01).
//!
//! Owns:
//! - [`BoardPostRequest`] payload coming from the frontend
//! - [`AppRuntime::post_board_entry_events`] impl extension (validates the
//!   target window, resolves audience, and persists the entry through
//!   `gwt_core::coordination::post_entry`)
//!
//! SPEC-3046: entry shape rules (body validation, list sanitization, origin
//! trimming) live in `gwt_core::coordination::BoardEntryDraft::finalize`,
//! shared with the CLI posting surface. SPEC-1974 Phase 9 / Phase 10
//! contracts (`target_owners`, `>>` marker, reminder coordination axes)
//! flow through here unchanged — the handler emits the same
//! `BackendEvent::BoardEntries` / `BackendEvent::BoardError` responses.

use std::path::Path;

use gwt_agent::{AgentId, AgentLaunchBuilder, LaunchConfig, SessionMode};
use gwt_core::{
    coordination::{self, BoardEntryKind, BoardMention},
    workspace_projection,
};

use gwt::board_audience::{gui_default_board_scope, post_audience_for_gui};

use super::{
    combined_window_id, same_worktree_path, AppRuntime, BackendEvent, OutboundEvent,
    WindowGeometry, WindowPreset,
};

pub(super) fn gui_default_board_scope_for_project(
    project_root: &Path,
) -> gwt_core::Result<coordination::BoardAudienceScope> {
    gui_default_board_scope(project_root)
}

#[derive(Debug, Clone)]
pub struct BoardPostRequest {
    pub(crate) id: String,
    pub(crate) entry_kind: BoardEntryKind,
    pub(crate) body: String,
    /// SPEC-2963: optional post title/subject from the composer.
    pub(crate) title: Option<String>,
    pub(crate) parent_id: Option<String>,
    pub(crate) topics: Vec<String>,
    pub(crate) owners: Vec<String>,
    pub(crate) targets: Vec<String>,
    pub(crate) mentions: Vec<BoardMention>,
    /// SPEC-2959: composer "To:" target Work (workspace id), or `None` for the
    /// active-workspace default.
    pub(crate) target_workspace: Option<String>,
    /// SPEC-2959: post to the General lane (broadcast, empty audience).
    pub(crate) broadcast: bool,
}

impl AppRuntime {
    pub(crate) fn post_board_entry_events(
        &mut self,
        client_id: &str,
        request: BoardPostRequest,
    ) -> Vec<OutboundEvent> {
        let BoardPostRequest {
            id,
            entry_kind,
            body,
            title,
            parent_id,
            topics,
            owners,
            targets,
            mentions,
            target_workspace,
            broadcast,
        } = request;

        let Some(address) = self.window_lookup.get(&id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let tab_id = tab.id.clone();
        let project_root = tab.project_root.clone();
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Board {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window is not a Board surface".to_string(),
                },
            )];
        }

        let mentions = coordination::normalize_board_mentions(&mentions);
        let audience = match post_audience_for_gui(
            &tab.project_root,
            &mentions,
            target_workspace.as_deref(),
            broadcast,
        ) {
            Ok(audience) => audience,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardError {
                        id,
                        message: error.to_string(),
                    },
                )];
            }
        };

        // SPEC-3046: エントリの形を決める正規化・検証は
        // BoardEntryDraft::finalize に集約されている。GUI 側は author
        // (User/"You") / audience 解決 / parent 存在検証 (IO) だけを担う。
        let mut draft = coordination::BoardEntryDraft::new(
            coordination::AuthorKind::User,
            "You",
            entry_kind,
            body,
        );
        draft.title = title;
        draft.parent_id = parent_id;
        draft.related_topics = topics;
        draft.related_owners = owners;
        draft.target_owners = targets;
        draft.mentions = mentions;
        if let Some(audience) = audience {
            draft.audience = audience;
        }
        let entry = match draft.finalize() {
            Ok(entry) => entry,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardError {
                        id,
                        message: error.to_string(),
                    },
                )];
            }
        };

        if let Some(parent_id) = entry.parent_id.as_deref() {
            let parent_exists =
                match gwt::board_provider::board_entry_exists(&tab.project_root, parent_id) {
                    Ok(parent_exists) => parent_exists,
                    Err(error) => {
                        return vec![OutboundEvent::reply(
                            client_id,
                            BackendEvent::BoardError {
                                id,
                                message: error.to_string(),
                            },
                        )];
                    }
                };
            if !parent_exists {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardError {
                        id,
                        message: "Reply target was not found".to_string(),
                    },
                )];
            }
        }

        match gwt::board_provider::post_entry(&tab.project_root, entry) {
            Ok(snapshot) => {
                publish_board_change(&tab.project_root, snapshot.board.entries.len());
                let mut entries = snapshot.board.entries;
                // Capture the milestone entry before attaching the serialize-only
                // `body_html`, so workspace milestone persistence stays clean.
                let latest_entry = entries.last().cloned();
                attach_board_body_html(&mut entries);
                let mut events = vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardEntries {
                        id,
                        entries,
                        has_more_before: snapshot.board.has_more_before,
                    },
                )];
                if let Some(entry) = latest_entry.as_ref() {
                    events.extend(self.record_workspace_board_milestone_event(
                        &tab_id,
                        &project_root,
                        entry,
                    ));
                }
                events
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(crate) fn open_board_origin_agent_events(
        &mut self,
        client_id: &str,
        id: &str,
        origin_session_id: &str,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let (tab_id, board_geometry) = match self.board_surface_context(id) {
            Ok(context) => context,
            Err(message) => return board_error(client_id, id, message),
        };
        let origin_session_id = origin_session_id.trim();
        if origin_session_id.is_empty() {
            return board_error(client_id, id, "Board origin session is unavailable");
        }

        let live_window_id = self
            .active_agent_sessions
            .iter()
            .find(|(_, session)| session.session_id == origin_session_id)
            .map(|(window_id, _)| window_id.clone());
        if let Some(window_id) = live_window_id {
            let events = self.focus_window_events(&window_id, bounds);
            return if events.is_empty() {
                board_error(
                    client_id,
                    id,
                    format!("Board origin Agent window not found for {origin_session_id}"),
                )
            } else {
                events
            };
        }

        let config = match self.board_origin_agent_resume_config(origin_session_id) {
            Ok(config) => config,
            Err(message) => return board_error(client_id, id, message),
        };
        match self.spawn_agent_window(&tab_id, config, bounds.unwrap_or(board_geometry), None) {
            Ok(events) => events,
            Err(message) => board_error(client_id, id, message),
        }
    }

    pub(crate) fn board_origin_agent_resume_config(
        &self,
        origin_session_id: &str,
    ) -> Result<LaunchConfig, String> {
        let origin_session_id = origin_session_id.trim();
        if origin_session_id.is_empty() {
            return Err("Board origin session is unavailable".to_string());
        }
        let session_path = self.sessions_dir.join(format!("{origin_session_id}.toml"));
        let session = gwt_agent::Session::load_and_migrate(&session_path).map_err(|error| {
            format!("Board origin session {origin_session_id} could not be loaded: {error}")
        })?;
        if !session.supports_exact_session_resume() {
            return Err(format!(
                "Board origin session {origin_session_id} does not support exact resume"
            ));
        }
        let resume_session_id = session.exact_resume_session_id().ok_or_else(|| {
            format!("Board origin session {origin_session_id} has no agent session id")
        })?;

        let mut builder = AgentLaunchBuilder::new(session.agent_id.clone())
            .working_dir(session.worktree_path.clone())
            .branch(session.branch.clone())
            .session_mode(SessionMode::Resume)
            .resume_session_id(resume_session_id.to_string())
            .runtime_target(session.runtime_target)
            .docker_lifecycle_intent(session.docker_lifecycle_intent);

        if let Some(custom_agent) = self
            .launch_wizard_cache
            .agent_options()
            .into_iter()
            .find(|option| agent_option_matches_session(option, &session.agent_id))
            .and_then(|option| option.custom_agent)
        {
            builder = builder.custom_agent(custom_agent);
        }
        if let Some(model) = non_empty(session.model.as_deref()) {
            builder = builder.model(model.to_string());
        }
        if let Some(tool_version) = non_empty(session.tool_version.as_deref()) {
            builder = builder.version(tool_version.to_string());
        }
        if let Some(reasoning_level) = non_empty(session.reasoning_level.as_deref()) {
            builder = builder.reasoning_level(reasoning_level.to_string());
        }
        if session.skip_permissions {
            builder = builder.skip_permissions(true);
        }
        if session.fast_mode_enabled() {
            builder = builder.fast_mode(true);
        }
        if let Some(docker_service) = non_empty(session.docker_service.as_deref()) {
            builder = builder.docker_service(docker_service.to_string());
        }
        if let Some(linked_issue_number) = session.linked_issue_number {
            builder = builder.linked_issue_number(linked_issue_number);
        }
        if let Some(windows_shell) = session.windows_shell {
            builder = builder.windows_shell(windows_shell);
        }

        let mut config = builder.build();
        if !session.display_name.trim().is_empty() {
            config.display_name = session.display_name;
        }
        Ok(config)
    }

    fn board_surface_context(&self, id: &str) -> Result<(String, WindowGeometry), String> {
        let address = self
            .window_lookup
            .get(id)
            .ok_or_else(|| "Window not found".to_string())?;
        let tab = self
            .tab(&address.tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let window = tab
            .workspace
            .window(&address.raw_id)
            .ok_or_else(|| "Window not found".to_string())?;
        if window.preset != WindowPreset::Board {
            return Err("Window is not a Board surface".to_string());
        }
        Ok((address.tab_id.clone(), window.geometry.clone()))
    }

    pub(crate) fn record_workspace_board_milestone_event(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        entry: &coordination::BoardEntry,
    ) -> Vec<OutboundEvent> {
        let _ = tab_id;
        let projection = match workspace_projection::transact_workspace_state(
            project_root,
            |projection, work_items, _work_items_persisted| {
                let event =
                    workspace_projection::workspace_work_event_from_board_entry(projection, entry);
                let state_cutoff = work_items
                    .work_items
                    .iter()
                    .find(|item| item.id == event.work_item_id)
                    .map(|item| item.updated_at);
                let event_is_current = state_cutoff.is_none_or(|cutoff| event.updated_at >= cutoff);
                projection.record_board_milestone_with_state_cutoff(entry, state_cutoff);
                let events = (event_is_current
                    && board_entry_origin_can_record_workspace_work_event(projection, entry))
                .then_some(event)
                .into_iter()
                .collect();
                Ok((projection.clone(), events))
            },
        ) {
            Ok(projection) => projection,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    project_root = %project_root.display(),
                    "failed to persist workspace board milestone"
                );
                return Vec::new();
            }
        };

        self.apply_workspace_projection_title_sync(project_root, &projection)
    }
}

fn board_entry_origin_can_record_workspace_work_event(
    projection: &workspace_projection::WorkspaceProjection,
    entry: &coordination::BoardEntry,
) -> bool {
    let Some(session_id) = entry.origin_session_id.as_deref() else {
        return true;
    };
    projection
        .latest_agent_for_session(session_id)
        .is_some_and(|agent| agent.is_assigned() && entry.updated_at >= agent.updated_at)
}

fn board_error(client_id: &str, id: &str, message: impl Into<String>) -> Vec<OutboundEvent> {
    vec![OutboundEvent::reply(
        client_id,
        BackendEvent::BoardError {
            id: id.to_string(),
            message: message.into(),
        },
    )]
}

fn agent_option_matches_session(option: &gwt::AgentOption, agent_id: &AgentId) -> bool {
    let command_matches = option.id == agent_id.command();
    match agent_id {
        AgentId::Custom(id) => {
            command_matches
                || option
                    .custom_agent
                    .as_ref()
                    .map(|agent| agent.id == *id)
                    .unwrap_or(false)
        }
        _ => command_matches,
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Best-effort fan-out of a Board projection change to other gwt
/// instances connected to the same daemon (SPEC-2077 Phase H1).
///
/// This is a side-channel notification: the local file watcher already
/// triggers `UserEvent::BoardProjectionChanged` for the in-process GUI,
/// and the on-disk projection remains the source of truth. The daemon
/// publish gives **other** gwt instances on the same project a
/// deterministic push (instead of relying on each instance's file
/// watcher debounce). Any error is logged at debug level and ignored.
#[cfg(unix)]
fn publish_board_change(project_root: &std::path::Path, entries_count: usize) {
    // Fire-and-forget: spawn a detached thread so the GUI handler's
    // `Vec<OutboundEvent>` return is never delayed by the daemon
    // round-trip. The publish itself is bounded by the
    // `daemon_publisher::publish_event` per-stage timeout (~200 ms
    // each across connect / send / ack, ~600 ms worst case), so the
    // spawned thread can never linger long.
    let project_root_owned = project_root.to_path_buf();
    let _ = std::thread::Builder::new()
        .name("gwt-board-daemon-publish".to_string())
        .spawn(move || {
            let result = gwt::daemon_publisher::publish_event(
                &project_root_owned,
                "board",
                serde_json::json!({"entries_count": entries_count}),
            );
            if let Err(err) = result {
                tracing::debug!(
                    error = %err,
                    project_root = %project_root_owned.display(),
                    entries_count,
                    "board projection daemon publish failed (non-fatal)"
                );
            }
        });
}

#[cfg(not(unix))]
fn publish_board_change(_project_root: &std::path::Path, _entries_count: usize) {
    // Daemon publishing is gated on Unix; the local file watcher
    // continues to drive single-instance updates on other platforms.
}

/// Populate the serialize-only `body_html` display field on each entry from its
/// Markdown `body` (SPEC-2963). Called at every `BoardEntries` emit boundary so
/// the web UI renders server-sanitized HTML, while the JSONL event log (which
/// `skip_serializing`s a `None` `body_html`) never stores it.
pub(crate) fn attach_board_body_html(entries: &mut [coordination::BoardEntry]) {
    for entry in entries.iter_mut() {
        entry.body_html = Some(gwt::board_remote::markdown::markdown_to_html(&entry.body));
    }
}

impl AppRuntime {
    pub(crate) fn load_board_events(
        &mut self,
        client_id: &str,
        id: &str,
        all: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Board {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window is not a Board surface".to_string(),
                },
            )];
        }
        let project_root = tab.project_root.clone();
        if all {
            self.board_all_view_windows.insert(id.to_string());
        } else {
            self.board_all_view_windows.remove(id);
        }

        let scope = if all {
            gwt_core::coordination::BoardAudienceScope::All
        } else {
            match gui_default_board_scope_for_project(&project_root) {
                Ok(scope) => scope,
                Err(error) => {
                    return vec![OutboundEvent::reply(
                        client_id,
                        BackendEvent::BoardError {
                            id: id.to_string(),
                            message: error.to_string(),
                        },
                    )];
                }
            }
        };
        let snapshot_result = if matches!(scope, gwt_core::coordination::BoardAudienceScope::All) {
            gwt::board_provider::load_snapshot(&project_root)
        } else {
            gwt::board_provider::load_snapshot_for_scope(&project_root, &scope)
        };
        match snapshot_result {
            Ok(snapshot) => {
                let mut entries = snapshot.board.entries;
                attach_board_body_html(&mut entries);
                vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardEntries {
                        id: id.to_string(),
                        entries,
                        has_more_before: snapshot.board.has_more_before,
                    },
                )]
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(crate) fn load_board_history_events(
        &mut self,
        client_id: &str,
        id: &str,
        before_entry_id: Option<&str>,
        limit: usize,
        all: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Board {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window is not a Board surface".to_string(),
                },
            )];
        }
        let project_root = tab.project_root.clone();
        if all {
            self.board_all_view_windows.insert(id.to_string());
        } else {
            self.board_all_view_windows.remove(id);
        }

        let scope = if all {
            gwt_core::coordination::BoardAudienceScope::All
        } else {
            match gui_default_board_scope_for_project(&project_root) {
                Ok(scope) => scope,
                Err(error) => {
                    return vec![OutboundEvent::reply(
                        client_id,
                        BackendEvent::BoardError {
                            id: id.to_string(),
                            message: error.to_string(),
                        },
                    )];
                }
            }
        };
        let page_result = if matches!(scope, gwt_core::coordination::BoardAudienceScope::All) {
            gwt::board_provider::load_entries_before(&project_root, before_entry_id, limit)
        } else {
            gwt::board_provider::load_entries_before_for_scope(
                &project_root,
                before_entry_id,
                limit,
                &scope,
            )
        };
        match page_result {
            Ok(page) => {
                let mut entries = page.entries;
                attach_board_body_html(&mut entries);
                vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardHistoryPage {
                        id: id.to_string(),
                        entries,
                        has_more_before: page.has_more_before,
                    },
                )]
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(crate) fn handle_board_projection_changed_events(
        &mut self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let Ok(snapshot) = gwt::board_provider::load_snapshot(project_root) else {
            return Vec::new();
        };

        let mut events = Vec::new();
        let latest_entry = snapshot.board.entries.last().cloned();
        for tab in &self.tabs {
            if !same_worktree_path(&tab.project_root, project_root) {
                continue;
            }
            for window in &tab.workspace.persisted().windows {
                if window.preset != WindowPreset::Board {
                    continue;
                }
                let window_id = combined_window_id(&tab.id, &window.id);
                let scope = if self.board_all_view_windows.contains(&window_id) {
                    gwt_core::coordination::BoardAudienceScope::All
                } else {
                    gui_default_board_scope_for_project(&tab.project_root)
                        .unwrap_or(gwt_core::coordination::BoardAudienceScope::All)
                };
                let board = if matches!(scope, gwt_core::coordination::BoardAudienceScope::All) {
                    snapshot.board.clone()
                } else {
                    gwt::board_provider::load_snapshot_for_scope(&tab.project_root, &scope)
                        .map(|snapshot| snapshot.board)
                        .unwrap_or_else(|_| snapshot.board.clone())
                };
                let mut entries = board.entries;
                attach_board_body_html(&mut entries);
                events.push(OutboundEvent::broadcast(BackendEvent::BoardEntries {
                    id: window_id,
                    entries,
                    has_more_before: board.has_more_before,
                }));
            }
        }
        if let Some(entry) = latest_entry.as_ref() {
            if let Some((tab_id, project_root)) = self
                .tabs
                .iter()
                .find(|tab| {
                    same_worktree_path(&tab.project_root, project_root)
                        && self.active_tab_id.as_deref() == Some(tab.id.as_str())
                })
                .map(|tab| (tab.id.clone(), tab.project_root.clone()))
            {
                events.extend(self.record_workspace_board_milestone_event(
                    &tab_id,
                    &project_root,
                    entry,
                ));
            }
        }
        events
    }
}

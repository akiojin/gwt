//! Board post handler split out of `app_runtime/mod.rs` for SPEC-2077 Phase A
//! (arch-review handoff, 2026-05-01).
//!
//! Owns:
//! - [`BoardPostRequest`] payload coming from the frontend
//! - [`AppRuntime::post_board_entry_events`] impl extension (validates the
//!   target window, sanitizes lists, and persists the entry through
//!   `gwt_core::coordination::post_entry`)
//! - [`sanitize_board_list`] helper that trims and de-duplicates string
//!   payloads
//!
//! SPEC-1974 Phase 9 / Phase 10 contracts (`target_owners`, `>>` marker,
//! reminder coordination axes) flow through here unchanged — the handler
//! still uses `BoardEntry::with_target_owners` from `gwt-core` and emits
//! the same `BackendEvent::BoardEntries` / `BackendEvent::BoardError`
//! responses.

use std::path::Path;

use gwt_agent::{AgentId, AgentLaunchBuilder, LaunchConfig, SessionMode};
use gwt_core::{
    coordination::{self, BoardEntryKind, BoardMention},
    workspace_projection,
};

use gwt::board_audience::{gui_default_board_scope, post_audience_for_gui};

use super::{AppRuntime, BackendEvent, OutboundEvent, WindowGeometry, WindowPreset};

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
    pub(crate) parent_id: Option<String>,
    pub(crate) topics: Vec<String>,
    pub(crate) owners: Vec<String>,
    pub(crate) targets: Vec<String>,
    pub(crate) mentions: Vec<BoardMention>,
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
            parent_id,
            topics,
            owners,
            targets,
            mentions,
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

        let trimmed_body = body.trim();
        if trimmed_body.is_empty() {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Board entry body is required".to_string(),
                },
            )];
        }

        let parent_id = parent_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let topics = sanitize_board_list(&topics);
        let owners = sanitize_board_list(&owners);
        let targets = sanitize_board_list(&targets);
        let mentions = coordination::normalize_board_mentions(&mentions);

        if let Some(parent_id) = parent_id.as_deref() {
            let parent_exists = match coordination::board_entry_exists(&tab.project_root, parent_id)
            {
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

        let mut entry = coordination::BoardEntry::new(
            coordination::AuthorKind::User,
            "You",
            entry_kind,
            trimmed_body,
            None,
            parent_id,
            topics,
            owners,
        );
        if !targets.is_empty() {
            entry = entry.with_target_owners(targets);
        }
        if !mentions.is_empty() {
            entry = entry.with_mentions(mentions);
        }
        let audience = match post_audience_for_gui(&tab.project_root, &entry.mentions) {
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
        if let Some(audience) = audience {
            entry = entry.with_audience(audience);
        }
        match coordination::post_entry(&tab.project_root, entry) {
            Ok(snapshot) => {
                publish_board_change(&tab.project_root, snapshot.board.entries.len());
                let latest_entry = snapshot.board.entries.last().cloned();
                let mut events = vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardEntries {
                        id,
                        entries: snapshot.board.entries,
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
            let mut events = self.restore_window_events(&window_id);
            events.extend(self.focus_window_events(&window_id, bounds));
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
        if session.codex_fast_mode {
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
        let mut projection =
            match workspace_projection::load_or_default_workspace_projection(project_root) {
                Ok(projection) => projection,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        project_root = %project_root.display(),
                        "failed to load workspace projection for board milestone"
                    );
                    return Vec::new();
                }
            };
        projection.record_board_milestone(entry);
        if let Err(error) =
            workspace_projection::save_workspace_projection(project_root, &projection)
        {
            tracing::warn!(
                error = %error,
                project_root = %project_root.display(),
                "failed to save workspace projection for board milestone"
            );
            return Vec::new();
        }
        if board_entry_origin_can_record_workspace_work_event(&projection, entry) {
            let work_event =
                workspace_projection::workspace_work_event_from_board_entry(&projection, entry);
            if let Err(error) =
                workspace_projection::record_workspace_work_event(project_root, work_event)
            {
                tracing::warn!(
                    error = %error,
                    project_root = %project_root.display(),
                    "failed to record workspace WorkItem event for board milestone"
                );
            }
        }

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
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
        .is_some_and(|agent| agent.is_assigned())
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

fn sanitize_board_list(values: &[String]) -> Vec<String> {
    let mut sanitized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || sanitized.iter().any(|item| item == trimmed) {
            continue;
        }
        sanitized.push(trimmed.to_string());
    }
    sanitized
}

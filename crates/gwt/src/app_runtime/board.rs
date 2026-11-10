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

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use gwt_agent::{AgentId, AgentLaunchBuilder, LaunchConfig, SessionMode};
use gwt_core::{
    coordination::{self, BoardEntryKind, BoardMention},
    workspace_projection,
};

use gwt::board_audience::{gui_default_board_scope, post_audience_for_gui};

use super::{
    combined_window_id, same_worktree_path, AppRuntime, BackendEvent, OutboundEvent, WindowPreset,
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

        let posted_id = entry.id.clone();
        match gwt::board_provider::post_entry(&tab.project_root, entry) {
            Ok(snapshot) => {
                publish_board_change(&tab.project_root, snapshot.board.entries.len());
                // Capture the milestone entry before attaching the serialize-only
                // `body_html`, so workspace milestone persistence stays clean.
                let latest_entry = snapshot.board.entries.last().cloned();
                // A future-dated or concurrent post may be last in the snapshot.
                // Remote providers may replace the draft ID: retain their prior
                // snapshot acknowledgement when no exact identity is available.
                let mut entries = match snapshot
                    .board
                    .entries
                    .iter()
                    .find(|entry| entry.id == posted_id)
                {
                    Some(entry) => vec![entry.clone()],
                    None => snapshot.board.entries,
                };
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
    ) -> Vec<OutboundEvent> {
        let tab_id = match self.board_surface_context(id) {
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
            let events = self.focus_window_events(&window_id);
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
        match self.spawn_agent_window(&tab_id, config, None) {
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
            .predecessor_session_id(session.id.clone())
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

    fn board_surface_context(&self, id: &str) -> Result<String, String> {
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
        Ok(address.tab_id.clone())
    }

    pub(crate) fn record_workspace_board_milestone_event(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        entry: &coordination::BoardEntry,
    ) -> Vec<OutboundEvent> {
        let _ = tab_id;
        let projection = match persist_workspace_board_milestone(project_root, entry) {
            Ok((projection, _)) => projection,
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
    static CACHE: std::sync::OnceLock<std::sync::Mutex<BoardBodyHtmlCache>> =
        std::sync::OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for entry in entries.iter_mut() {
        entry.body_html =
            Some(cache.html_for(entry, gwt::board_remote::markdown::markdown_to_html));
    }
}

/// Bound the display-only memo independently of the persisted Board history.
/// Keep the source too: remote providers can reuse IDs in different channels.
#[derive(Default)]
struct BoardBodyHtmlCache {
    entries: HashMap<String, (chrono::DateTime<chrono::Utc>, String, String)>,
}

impl BoardBodyHtmlCache {
    fn html_for(
        &mut self,
        entry: &coordination::BoardEntry,
        render: impl FnOnce(&str) -> String,
    ) -> String {
        if let Some((updated_at, body, html)) = self.entries.get(&entry.id) {
            if *updated_at == entry.updated_at && *body == entry.body {
                return html.clone();
            }
        }
        let html = render(&entry.body);
        if self.entries.len() >= coordination::HOT_PROJECTION_ENTRY_LIMIT * 4 {
            self.entries.clear();
        }
        self.entries.insert(
            entry.id.clone(),
            (entry.updated_at, entry.body.clone(), html.clone()),
        );
        html
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

    /// Issue #4406: capture what a Board refresh needs. This is the only part
    /// of a Board change that runs on the GUI event loop; reading the Board
    /// there blocked it for 14s at the median once the Board held 3,410 posts.
    pub(crate) fn board_projection_refresh_job(
        &self,
        project_root: &Path,
    ) -> BoardProjectionRefreshJob {
        let mut windows = Vec::new();
        for tab in &self.tabs {
            if !same_worktree_path(&tab.project_root, project_root) {
                continue;
            }
            for window in &tab.workspace.persisted().windows {
                if window.preset != WindowPreset::Board {
                    continue;
                }
                let window_id = combined_window_id(&tab.id, &window.id);
                windows.push(BoardRefreshWindow {
                    all_view: self.board_all_view_windows.contains(&window_id),
                    window_id,
                    tab_project_root: tab.project_root.clone(),
                });
            }
        }
        let milestone_root = self
            .tabs
            .iter()
            .find(|tab| same_worktree_path(&tab.project_root, project_root))
            .map(|tab| tab.project_root.clone());
        BoardProjectionRefreshJob {
            context: self
                .tabs
                .iter()
                .find(|tab| same_worktree_path(&tab.project_root, project_root))
                .and_then(|tab| self.project_context(&tab.id)),
            project_root: project_root.to_path_buf(),
            windows,
            milestone_root,
        }
    }

    /// Issue #4406: apply a Board refresh that ran off the GUI event loop.
    /// A Work milestone is merged into the cached Active Work view, the same
    /// cache-only path the Workspace watcher takes (Issue #3783); rebuilding
    /// Active Work here cost seconds per Board post.
    pub(crate) fn apply_board_projection_refresh(
        &mut self,
        refreshed: BoardProjectionRefreshed,
    ) -> Vec<OutboundEvent> {
        let BoardProjectionRefreshed {
            context,
            mut events,
            milestone,
            decision_notifications,
        } = refreshed;
        if !context
            .as_ref()
            .is_some_and(|context| self.project_context_is_current(context))
        {
            return Vec::new();
        }
        let context = context.as_ref().expect("current Board project context");
        for notification in decision_notifications {
            self.notify_board_decision(context, notification);
        }
        events.retain(|event| match &event.event {
            BackendEvent::BoardEntries { id, .. } => {
                self.project_key_for_window(id) == Some(&context.project_key)
            }
            _ => false,
        });
        if let Some((project_root, projection)) = milestone {
            events.extend(
                self.apply_workspace_projection_title_sync_cache_only(&project_root, &projection),
            );
        }
        events
    }

    /// Board notifications have internal authority to send only this fixed
    /// pointer to a saved decision. They never grant an agent PM input rights
    /// or turn the decision's untrusted body into a privileged prompt.
    fn notify_board_decision(
        &self,
        context: &super::ProjectContext,
        notification: BoardDecisionNotification,
    ) {
        let root = context.project_root.clone();
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all().build().map_err(|error| error.to_string())
                .and_then(|executor| executor.block_on(async {
                    let client = gwt::project_runtime::client::ProjectRuntimeClient::from_current_project(
                        &root, &gwt_core::paths::gwt_home(),
                    ).await?;
                    client.command(gwt::project_runtime::events::OwnerCommand::NotifyBoardDecision {
                        entry_id: notification.entry_id, session_id: notification.session_id,
                    }).await
                }));
            if let Err(error) = result {
                tracing::warn!(%error, "project owner declined Board notification");
            }
        }) {
            tracing::warn!(%error, "Board notification worker unavailable");
        }
    }

    #[cfg(test)]
    pub(crate) fn handle_board_projection_changed_events(
        &mut self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let job = self.board_projection_refresh_job(project_root);
        let (refreshed, _) = run_board_projection_refresh(job, BoardScopedViews::new());
        self.apply_board_projection_refresh(refreshed)
    }
}

/// Issue #4406: a Board change captured on the GUI event loop, to be refreshed
/// off it by [`run_board_projection_refresh`].
#[derive(Debug)]
pub(crate) struct BoardProjectionRefreshJob {
    pub(crate) context: Option<super::ProjectContext>,
    pub(crate) project_root: PathBuf,
    windows: Vec<BoardRefreshWindow>,
    /// Set for an open project: its latest post is recorded as
    /// a Work milestone under this root.
    milestone_root: Option<PathBuf>,
}

#[derive(Debug)]
struct BoardRefreshWindow {
    window_id: String,
    tab_project_root: PathBuf,
    all_view: bool,
}

/// The outcome of [`run_board_projection_refresh`], applied on the GUI event
/// loop by [`AppRuntime::apply_board_projection_refresh`].
#[derive(Debug, Clone, Default)]
pub(crate) struct BoardProjectionRefreshed {
    pub(crate) context: Option<super::ProjectContext>,
    pub(crate) events: Vec<OutboundEvent>,
    /// The Workspace projection after the latest post was recorded as a Work
    /// milestone, when it resolved to one.
    pub(crate) milestone: Option<(PathBuf, workspace_projection::WorkspaceProjection)>,
    /// Saved decision pointers, independent of whether any Board window is open.
    pub(crate) decision_notifications: Vec<BoardDecisionNotification>,
}

#[derive(Debug, Clone)]
pub(crate) struct BoardDecisionNotification {
    entry_id: String,
    session_id: String,
}

/// Scoped Board views of one project, by Board window, kept between refreshes
/// so a post is applied incrementally instead of rescanning the history.
pub(crate) type BoardScopedViews = HashMap<String, coordination::ScopedBoardView>;

/// Issue #4406: refresh every Board window of a project. Runs off the GUI
/// event loop; the views of windows that no longer exist are dropped.
pub(crate) fn run_board_projection_refresh(
    job: BoardProjectionRefreshJob,
    mut views: BoardScopedViews,
) -> (BoardProjectionRefreshed, BoardScopedViews) {
    let mut refreshed = BoardProjectionRefreshed {
        context: job.context.clone(),
        ..Default::default()
    };
    let mut retained = BoardScopedViews::new();
    let Some(context) = job.context.as_ref() else {
        return (refreshed, retained);
    };
    let Ok(snapshot) = gwt::board_provider::load_snapshot(&job.project_root) else {
        return (refreshed, views);
    };
    refreshed.decision_notifications = snapshot
        .board
        .entries
        .iter()
        .filter(|entry| entry.kind == BoardEntryKind::Decision)
        .flat_map(|entry| {
            entry
                .mentions
                .iter()
                .filter(|mention| {
                    mention.target_kind == coordination::BoardMentionTargetKind::Session
                })
                .map(|mention| BoardDecisionNotification {
                    entry_id: entry.id.clone(),
                    session_id: mention.target.clone(),
                })
        })
        .collect();
    for window in job.windows {
        let scope = if window.all_view {
            coordination::BoardAudienceScope::All
        } else {
            let Ok(scope) = gui_default_board_scope_for_project(&window.tab_project_root) else {
                if let Some(previous) = views.remove(&window.window_id) {
                    retained.insert(window.window_id, previous);
                }
                continue;
            };
            scope
        };
        let previous = views
            .remove(&window.window_id)
            .filter(|view| view.scope == scope);
        let known = previous.as_ref().map(|view| {
            view.board
                .entries
                .iter()
                .map(|entry| (entry.id.as_str(), entry))
                .collect::<HashMap<_, _>>()
        });
        let previous_total = previous.as_ref().map(|view| view.board.total_entries);
        let since = previous.as_ref().map(|view| {
            view.board
                .entries
                .iter()
                .map(|entry| entry.updated_at)
                .max()
                .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
        });
        // All uses the same bounded cache as Workspace/Broadcast. The core
        // hot cursor handles ordinary appends without rereading event segments.
        let Ok((view, _)) = gwt::board_provider::refresh_scoped_board_view(
            &window.tab_project_root,
            &scope,
            previous.clone(),
        ) else {
            if let Some(previous) = previous {
                retained.insert(window.window_id, previous);
            }
            continue;
        };
        let changed = |entry: &coordination::BoardEntry| {
            // Some remote providers keep their creation timestamp on edits.
            !known
                .as_ref()
                .and_then(|known| known.get(entry.id.as_str()))
                .is_some_and(|known| *known == entry)
        };
        let mut entries = view
            .board
            .entries
            .iter()
            .filter(|entry| changed(entry))
            .cloned()
            .collect::<Vec<_>>();
        // A coalesced burst may exceed the hot window. Recover those new posts
        // via the timestamp-indexed segments, without adding I/O to normal posts.
        // Include the boundary timestamp; the ID/version comparison deduplicates
        // it, while the view difference also catches backdated entries.
        if previous_total.is_some_and(|total| {
            view.board.total_entries.saturating_sub(total) > view.board.entries.len()
        }) {
            if let Some(since) = since {
                let since = since
                    .checked_sub_signed(chrono::Duration::nanoseconds(1))
                    .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);
                match gwt::board_provider::load_entries_since(&window.tab_project_root, since) {
                    Ok(recent) => {
                        let mut ids = entries
                            .iter()
                            .map(|entry| entry.id.clone())
                            .collect::<std::collections::HashSet<_>>();
                        entries.extend(recent.into_iter().filter(|entry| {
                            coordination::board_entry_visible_for_scope(entry, &scope)
                                && changed(entry)
                                && ids.insert(entry.id.clone())
                        }));
                        entries.sort_by_key(|entry| entry.created_at);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to load Board catch-up entries");
                        if let Some(previous) = previous {
                            retained.insert(window.window_id, previous);
                        }
                        continue;
                    }
                }
            }
        }
        let has_more_before = view.board.has_more_before;
        retained.insert(window.window_id.clone(), view);
        if entries.is_empty() {
            continue;
        }
        attach_board_body_html(&mut entries);
        refreshed.events.push(OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::BoardEntries {
                id: window.window_id,
                entries,
                has_more_before,
            },
        ));
    }
    if let (Some(root), Some(entry)) = (job.milestone_root, snapshot.board.entries.last()) {
        match persist_workspace_board_milestone(&root, entry) {
            Ok((projection, true)) => refreshed.milestone = Some((root, projection)),
            Ok((_, false)) => {}
            Err(error) => tracing::warn!(
                error = %error,
                project_root = %root.display(),
                "failed to persist workspace board milestone"
            ),
        }
    }
    (refreshed, retained)
}

/// Record `entry` as a Work milestone. Returns the persisted Workspace
/// projection and whether the entry resolved to a Work event at all; when it
/// did not, nothing a Work row shows has changed.
fn persist_workspace_board_milestone(
    project_root: &Path,
    entry: &coordination::BoardEntry,
) -> gwt_core::Result<(workspace_projection::WorkspaceProjection, bool)> {
    let projection = workspace_projection::load_or_default_workspace_projection(project_root)?;
    let work_items = workspace_projection::load_or_synthesize_workspace_work_items(project_root)?;
    let work_event_root = workspace_projection::resolve_workspace_work_event_from_board_entry(
        &projection,
        &work_items,
        entry,
    )
    .and_then(|event| event.execution_container?.worktree_path)
    .unwrap_or_else(|| project_root.to_path_buf());
    workspace_projection::transact_workspace_state_for_work_event_root_with_preflight(
        project_root,
        &work_event_root,
        |projection, work_items, _work_items_persisted| {
            let current_root = workspace_projection::resolve_workspace_work_event_from_board_entry(
                projection, work_items, entry,
            )
            .and_then(|event| event.execution_container?.worktree_path)
            .unwrap_or_else(|| project_root.to_path_buf());
            if current_root != work_event_root {
                return Err(gwt_core::error::GwtError::Other(
                    "Board milestone Worktree assignment changed; retry with current state".into(),
                ));
            }
            Ok(())
        },
        |projection, work_items, _work_items_persisted| {
            // The locked preflight rejects changed roots before migration writes.
            let Some(event) = workspace_projection::resolve_workspace_work_event_from_board_entry(
                projection, work_items, entry,
            ) else {
                return Ok(((projection.clone(), false), Vec::new()));
            };
            let state_cutoff = work_items
                .work_items
                .iter()
                .find(|item| item.id == event.work_item_id)
                .map(|item| item.updated_at);
            if event.work_item_id == projection.id {
                projection.record_board_milestone_with_state_cutoff(entry, state_cutoff);
            }
            Ok(((projection.clone(), true), vec![event]))
        },
    )
}

#[cfg(test)]
mod body_html_tests {
    use super::*;

    #[test]
    fn board_body_html_memo_reuses_versions_and_invalidates_changed_bodies() {
        let mut cache = BoardBodyHtmlCache::default();
        let mut entry = coordination::BoardEntry::new(
            coordination::AuthorKind::Agent,
            "test",
            BoardEntryKind::Status,
            "**old**",
            None,
            None,
            vec![],
            vec![],
        );
        let first = cache.html_for(&entry, gwt::board_remote::markdown::markdown_to_html);
        assert_eq!(
            cache.html_for(&entry, |_| panic!("unchanged body rendered again")),
            first
        );
        entry.updated_at += chrono::Duration::seconds(1);
        entry.body = "**new**".into();
        let next = cache.html_for(&entry, gwt::board_remote::markdown::markdown_to_html);
        assert!(next.contains("<strong>new</strong>"));
        // The display cache is shared across providers: guard reused channel IDs.
        entry.body = "other channel".into();
        assert_eq!(cache.html_for(&entry, str::to_string), "other channel");
    }
}

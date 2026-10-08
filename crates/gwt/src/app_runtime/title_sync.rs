//! Canonical orchestration for syncing `WorkspaceProjection` title surfaces
//! (per-agent `title_summary` / `current_focus`) into in-memory window state
//! and emitting the consequent broadcasts in one batch.
//!
//! Background (SPEC-2359 US-26 / Phase U-1..U-4):
//! Before this module, every write path that mutated
//! `projection.agents[<i>].title_summary` had to remember each step:
//! (1) update `current.json` + `journal.jsonl`,
//! (2) sync `tab.workspace.windows[<id>].dynamic_title` in memory,
//! (3) broadcast `BackendEvent::ActiveWorkProjection`, and
//! (4) broadcast `BackendEvent::WindowCanvasState` so the pane heading
//! `windowData.dynamic_title` consumed by `windowDisplayTitle` on the
//! frontend refreshes. `workspace.update` with `params.purpose` ran (1)
//! and (3) but never (4), so the pane heading kept the `agent_id`
//! fallback ("CLAUDE CODE") even when `projection.agents[<i>].title_summary`
//! had a fresh value.
//!
//! `apply_workspace_projection_title_sync` consolidates (2)..(4) so any
//! caller that has just observed a projection change just dispatches the
//! returned `Vec<OutboundEvent>` and is guaranteed to leave the surfaces
//! consistent. Phase U-1 (this commit) keeps the broadcast surface
//! identical to the pre-refactor behavior to preserve every existing
//! test; Phase U-2 wires the `WindowCanvasState` broadcast in;
//! Phase U-3 adds `active_agent_sessions` backfill for sessions that
//! gwt's launch flow has not yet registered.

use std::{collections::HashMap, path::Path};

use gwt_core::workspace_projection::WorkspaceProjection;

use crate::same_worktree_path;

use super::{ActiveAgentSession, AppRuntime, OutboundEvent};

pub(crate) type WorkspaceWindowTitles = HashMap<String, (Option<String>, Option<String>)>;
pub(crate) type WorkspaceTitleUpdate = (String, Option<String>, Option<String>);

/// Resolve and compare titles from captured state, without accessing the GUI.
pub(crate) fn resolve_workspace_projection_title_updates(
    project_root: &Path,
    projection: &WorkspaceProjection,
    sessions: &[ActiveAgentSession],
    windows: &WorkspaceWindowTitles,
) -> Vec<WorkspaceTitleUpdate> {
    let normalized = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let issue_title = normalized(
        projection
            .linked_issues
            .first()
            .and_then(|issue| issue.title.as_deref()),
    );
    let mut current_titles = windows.clone();
    projection
        .agents
        .iter()
        .filter_map(|agent| {
            let window_id = resolve_title_sync_window_id(agent, project_root, sessions, windows)?;
            let title = normalized(agent.title_summary.as_deref()).or_else(|| issue_title.clone());
            let detail = normalized(agent.current_focus.as_deref());
            let current = current_titles.get_mut(&window_id)?;
            if *current == (title.clone(), detail.clone()) {
                return None;
            }
            *current = (title.clone(), detail.clone());
            Some((window_id, title, detail))
        })
        .collect()
}

fn resolve_title_sync_window_id(
    agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
    project_root: &Path,
    sessions: &[ActiveAgentSession],
    windows: &WorkspaceWindowTitles,
) -> Option<String> {
    // Session identity resolves across tabs; the worktree-only fallbacks must
    // belong to this project and must not choose between multiple sessions.
    if let Some(session) = sessions
        .iter()
        .find(|session| session.session_id == agent.session_id)
    {
        return Some(session.window_id.clone());
    }
    let worktree = agent.worktree_path.as_deref()?;
    if !same_worktree_path(worktree, project_root) {
        return None;
    }
    if let Some(window_id) = agent
        .window_id
        .as_ref()
        .filter(|id| windows.contains_key(*id))
    {
        return Some(window_id.clone());
    }
    let mut matches = sessions.iter().filter(|session| {
        same_worktree_path(&session.worktree_path, worktree) && session.agent_id == agent.agent_id
    });
    let window_id = matches.next()?.window_id.clone();
    matches.next().is_none().then_some(window_id)
}

impl AppRuntime {
    /// Run the canonical title-sync orchestration for the supplied projection.
    ///
    /// Side effects:
    /// - Mutates `tab.workspace.windows[<id>].dynamic_title` /
    ///   `dynamic_title_detail` from `projection.agents[<i>].title_summary`
    ///   / `current_focus` via
    ///   [`AppRuntime::sync_agent_window_titles_from_workspace_projection`].
    ///
    /// Return value:
    /// - The `OutboundEvent`s that callers should dispatch. Phase U-2
    ///   (SPEC-2359 US-26) makes this emit `BackendEvent::WindowCanvasState`
    ///   when an in-memory `dynamic_title` actually changed, so the
    ///   frontend's pane heading (`windowDisplayTitle` →
    ///   `windowData.dynamic_title`) updates immediately without waiting
    ///   for the next hook event or window structure change. The
    ///   `BackendEvent::ActiveWorkProjection` broadcast for the active tab
    ///   is emitted unconditionally — that surface refreshes the Active
    ///   Work card and Workspace Kanban entries regardless of whether a
    ///   pane heading was touched. The potentially large projection is
    ///   prepared and serialized on the blocking worker, then broadcast by
    ///   the tao continuation instead of being returned in this event batch.
    pub(crate) fn apply_workspace_projection_title_sync(
        &mut self,
        project_root: &Path,
        projection: &WorkspaceProjection,
    ) -> Vec<OutboundEvent> {
        self.apply_workspace_projection_title_sync_with_mode(project_root, projection, false)
    }

    pub(crate) fn apply_workspace_projection_title_sync_cache_only(
        &mut self,
        project_root: &Path,
        projection: &WorkspaceProjection,
    ) -> Vec<OutboundEvent> {
        self.apply_workspace_projection_title_sync_with_mode(project_root, projection, true)
    }

    fn apply_workspace_projection_title_sync_with_mode(
        &mut self,
        project_root: &Path,
        projection: &WorkspaceProjection,
        cache_only: bool,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_root(project_root) else {
            return Vec::new();
        };
        let dynamic_title_changed =
            self.sync_agent_window_titles_from_workspace_projection(project_root, projection);

        let mut events = Vec::new();
        if dynamic_title_changed {
            events.push(self.workspace_state_broadcast(&context));
        }
        let projection_event = if cache_only {
            // Cache-only callers merge the supplied snapshot into the existing
            // view rather than rebuilding Session/WorkItems history. Watcher
            // notifications prepare this merge on a worker (Issue #5118).
            self.merge_workspace_projection_into_cached_active_work(project_root, projection);
            self.cached_active_work_projection_broadcast_for_workspace_watcher(&context.tab_id)
        } else {
            self.active_work_projection_broadcast_for_tab(&context.tab_id)
        };
        if let Some(event) = projection_event {
            events.push(event);
        }
        events
    }

    /// Sync `projection.agents[<i>].title_summary` / `current_focus` into the
    /// matching `tab.workspace.windows[<id>].dynamic_title` /
    /// `dynamic_title_detail`. Returns `true` if at least one window was
    /// touched.
    ///
    /// Callers should generally go through
    /// [`AppRuntime::apply_workspace_projection_title_sync`] (Phase U-1+)
    /// rather than calling this directly, so that the consequent broadcasts
    /// are emitted in the same batch.
    pub(crate) fn sync_agent_window_titles_from_workspace_projection(
        &mut self,
        project_root: &Path,
        projection: &gwt_core::workspace_projection::WorkspaceProjection,
    ) -> bool {
        let sessions = self
            .active_agent_sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let updates = resolve_workspace_projection_title_updates(
            project_root,
            projection,
            &sessions,
            &self.workspace_projection_title_windows(),
        );

        let mut changed = false;
        for (window_id, title, detail) in updates {
            let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                continue;
            };
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                continue;
            };
            if tab
                .workspace
                .set_dynamic_title_with_detail(&address.raw_id, title, detail)
            {
                self.invalidate_workspace_projection_patch(&address.tab_id);
                changed = true;
            }
        }
        changed
    }

    /// Capture every window, including another tab reached by session identity.
    pub(crate) fn workspace_projection_title_windows(&self) -> WorkspaceWindowTitles {
        self.window_lookup
            .iter()
            .filter_map(|(id, address)| {
                let window = self
                    .tab(&address.tab_id)?
                    .workspace
                    .window(&address.raw_id)?;
                Some((
                    id.clone(),
                    (
                        window.dynamic_title.clone(),
                        window.dynamic_title_detail.clone(),
                    ),
                ))
            })
            .collect()
    }
}
